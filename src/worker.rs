use crate::{
    db::{self, Host, Snapshot},
    ssh::{Session, protect},
};
use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use serde_json::json;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub enum Action {
    Poll,
    Catalog,
    Rules,
    Manage { action: &'static str, value: String },
    Update,
    Remove,
    Stop,
}
pub struct Worker {
    pub tx: Sender<Action>,
    pub status: Receiver<String>,
    pub results: Receiver<Result<String, String>>,
    pub catalogs: Receiver<(bool, Result<crate::catalog::Catalog, String>)>,
}
pub fn launch(path: PathBuf, h: Host) -> Worker {
    let (tx, rx) = mpsc::channel();
    let (report, status) = mpsc::channel();
    let (finished, results) = mpsc::channel();
    let (catalog_tx, catalogs) = mpsc::channel();
    thread::spawn(move || {
        if let Err(e) = run(path, &h, &rx, &report, &finished, &catalog_tx) {
            let _ = report.send(format!("{e:#}"));
        }
    });
    Worker {
        tx,
        status,
        results,
        catalogs,
    }
}
fn run(
    path: PathBuf,
    h: &Host,
    rx: &Receiver<Action>,
    out: &Sender<String>,
    finished: &Sender<Result<String, String>>,
    catalogs: &Sender<(bool, Result<crate::catalog::Catalog, String>)>,
) -> Result<()> {
    let mut db = db::open(&path)?;
    let key = protect(&h.secret, false)?;
    let mut session = None;
    let mut next = Instant::now();
    loop {
        let action = match rx.recv_timeout(next.saturating_duration_since(Instant::now())) {
            Ok(Action::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => Action::Poll,
            Ok(a) => a,
        };
        let removal = matches!(action, Action::Remove);
        let managing = matches!(action, Action::Manage { .. });
        let mut archiving = false;
        let result = (|| -> Result<()> {
            if session.is_none() {
                out.send("Подключение…".into())?;
                session = Some(Session::start(h, &key, "panel session")?);
            }
            let ssh = session.as_mut().unwrap();
            match &action {
                Action::Catalog => {
                    let mut entries = crate::catalog::Catalog::new();
                    for (base, action) in [(true, "base-list"), (false, "list")] {
                        let data = ssh.request(json!({"op":"manage","action":action}))?;
                        entries.extend(crate::catalog::parse(
                            data["message"].as_str().unwrap_or(""),
                            base,
                        )?);
                    }
                    catalogs.send((false, Ok(entries)))?;
                }
                Action::Rules => {
                    let mut entries = crate::catalog::Catalog::new();
                    for (exception, op) in [(false, "ban-list"), (true, "exception-list")] {
                        let data = ssh.request(json!({"op":"manage","action":op}))?;
                        entries.extend(crate::catalog::rules(
                            data["message"].as_str().unwrap_or(""),
                            exception,
                        )?);
                    }
                    catalogs.send((true, Ok(entries)))?;
                }
                Action::Manage { action, value } => {
                    out.send("Выполнение команды…".into())?;
                    let reply =
                        ssh.request(json!({"op":"manage","action":action,"value":value}))?;
                    let message = reply["message"]
                        .as_str()
                        .context("Сервер не вернул результат действия")?;
                    finished.send(Ok(message.into()))?;
                    out.send("На связи · 100%".into())?;
                }
                Action::Poll => {
                    out.send("Опрос · 25%".into())?;
                    let cursor: Option<String> = db
                        .query_row(
                            "SELECT cursor FROM panel_state WHERE host=?",
                            [&h.name],
                            |r| r.get(0),
                        )
                        .optional()?;
                    let archive: Option<Option<String>> = db
                        .query_row(
                            "SELECT cursor FROM panel_log_archive WHERE host=?",
                            [&h.name],
                            |r| r.get(0),
                        )
                        .optional()?;
                    let mut request = json!({"op":"snapshot","cursor":cursor.unwrap_or_default()});
                    if let Some(cursor) = archive.unwrap_or_else(|| Some(String::new())) {
                        request["archive_cursor"] = json!(cursor);
                    }
                    let snapshot = serde_json::from_value::<Snapshot>(ssh.request(request)?)
                        .context("Формат замера")?;
                    archiving = snapshot
                        .log_archive
                        .as_ref()
                        .is_some_and(|archive| archive.cursor.is_some());
                    out.send("Опрос · 75%".into())?;
                    db::save(&mut db, h, snapshot)?;
                    out.send("На связи · 100%".into())?;
                }
                Action::Update => {
                    let mut stage = "Ждём первый ответ сервера";
                    out.send(format!("Обновление · {stage}"))?;
                    ssh.request_with_progress(json!({"op":"update"}), |code| {
                        stage = match code {
                            "prepare" => "Подготовка обновления",
                            "clone" => "Загрузка исходников с GitHub",
                            "build" => "Сборка rkn-guard",
                            "install" => "Установка бинарника и меню",
                            "logging" => "Настройка журнала",
                            "heartbeat" => stage,
                            _ => return,
                        };
                        let _ = out.send(format!("Обновление · {stage}"));
                    })?;
                    out.send("Обновлено · 100%".into())?;
                }
                Action::Remove => {
                    out.send("Удаление · 25%".into())?;
                    ssh.request(json!({"op":"uninstall","confirm":true}))?;
                    db.execute("UPDATE panel_hosts SET active=0 WHERE name=?", [&h.name])?;
                    out.send("rkn-guard удалён · 100%".into())?;
                }
                Action::Stop => {}
            }
            Ok(())
        })();
        let updated = result.is_ok() && matches!(action, Action::Update);
        if let Err(e) = result {
            session = None;
            let msg = format!("{e:#}");
            if matches!(action, Action::Catalog | Action::Rules) {
                catalogs.send((matches!(action, Action::Rules), Err(msg.clone())))?;
            }
            if managing {
                let message = if msg.contains("Неизвестная операция панели")
                {
                    "Обновите rkn-guard до версии dev с управлением из панели.".to_string()
                } else {
                    msg.clone()
                };
                finished.send(Err(message))?;
            }
            db.execute(
                "INSERT INTO polls VALUES(?,strftime('%s','now'),'error',?)",
                rusqlite::params![h.name, msg],
            )?;
            out.send(msg)?;
        } else if removal {
            break;
        }
        next = Instant::now() + Duration::from_secs(if updated || archiving { 2 } else { 30 });
    }
    Ok(())
}
