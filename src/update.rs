use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use std::{
    ffi::OsStr,
    fs,
    os::windows::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const CHECK: &str = include_str!("update/check.ps1");
const DOWNLOAD: &str = include_str!("update/download.ps1");
const REPLACE: &str = include_str!("update/replace.ps1");

#[derive(Deserialize)]
pub struct Release {
    tag_name: String,
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    digest: Option<String>,
    size: u64,
}

pub struct Update {
    pub tag: String,
    sha256: String,
    size: u64,
}

pub enum Check {
    Current,
    Missing,
    NoAsset(String),
    Available(Update),
}

fn version(tag: &str) -> Option<[u32; 3]> {
    let mut parts = tag.strip_prefix('v').unwrap_or(tag).split('.');
    let number = |part: &str| {
        (!part.is_empty() && part.bytes().all(|c| c.is_ascii_digit()))
            .then(|| part.parse().ok())
            .flatten()
    };
    let numbers = [
        number(parts.next()?)?,
        number(parts.next()?)?,
        number(parts.next()?)?,
    ];
    parts.next().is_none().then_some(numbers)
}

fn evaluate(release: Release) -> Result<Check> {
    let remote = version(&release.tag_name).context("Неверный тег релиза GitHub")?;
    let local = version(VERSION).context("Неверная версия приложения")?;
    if remote <= local {
        return Ok(Check::Current);
    }
    let Some(asset) = release
        .assets
        .iter()
        .find(|asset| asset.name == "rkn-guard-panel.exe")
    else {
        return Ok(Check::NoAsset(release.tag_name));
    };
    let sha256 = asset
        .digest
        .as_deref()
        .and_then(|value| value.strip_prefix("sha256:"))
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()));
    let Some(hash) = sha256 else {
        return Ok(Check::NoAsset(release.tag_name));
    };
    ensure!(
        (1..=128 * 1024 * 1024).contains(&asset.size),
        "Недопустимый размер EXE"
    );
    Ok(Check::Available(Update {
        tag: release.tag_name,
        sha256: hash.into(),
        size: asset.size,
    }))
}

fn powershell(script: &str) -> Result<Command> {
    let root = std::env::var_os("SystemRoot").context("Не найден каталог Windows")?;
    let executable = PathBuf::from(root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let encoded = STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let mut cmd = Command::new(executable);
    cmd.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded]);
    cmd.creation_flags(0x0800_0000);
    Ok(cmd)
}

fn run(cmd: &mut Command) -> Result<Vec<u8>> {
    let output = cmd
        .output()
        .context("Не удалось запустить обновление Windows PowerShell")?;
    ensure!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output.stdout)
}

pub fn check() -> Result<Check> {
    let raw = run(&mut powershell(CHECK)?)?;
    let release: Option<Release> =
        serde_json::from_slice(&raw).context("Не удалось прочитать ответ GitHub")?;
    release
        .map(evaluate)
        .transpose()
        .map(|value| value.unwrap_or(Check::Missing))
}

pub struct Staged {
    directory: PathBuf,
    target: PathBuf,
    sha256: String,
    handed_off: bool,
}

impl Drop for Staged {
    fn drop(&mut self) {
        if !self.handed_off {
            let _ = fs::remove_file(self.directory.join("new.exe"));
            let _ = fs::remove_dir(&self.directory);
        }
    }
}

impl Staged {
    pub fn launch(mut self) -> Result<()> {
        powershell(REPLACE)?
            .env("RKN_UPDATE_DIR", &self.directory)
            .env("RKN_UPDATE_TARGET", &self.target)
            .env("RKN_UPDATE_SHA256", &self.sha256)
            .env("RKN_UPDATE_PID", std::process::id().to_string())
            .spawn()
            .context("Не удалось запустить замену EXE")?;
        self.handed_off = true;
        Ok(())
    }
}

pub fn download(update: Update) -> Result<Staged> {
    let target = std::env::current_exe()?;
    let parent = target.parent().context("Не найден каталог приложения")?;
    let directory = parent.join(format!(".rkn-update-{:016x}", rand::random::<u64>()));
    fs::create_dir(&directory).context("Нет доступа на запись в каталог EXE")?;
    let staged = Staged {
        directory,
        target,
        sha256: update.sha256,
        handed_off: false,
    };
    let url = format!(
        "https://github.com/Flecksis/rkn-guard-panel/releases/download/{}/rkn-guard-panel.exe",
        update.tag
    );
    run(powershell(DOWNLOAD)?
        .env("RKN_UPDATE_URL", url)
        .env("RKN_UPDATE_FILE", staged.directory.join("new.exe"))
        .env("RKN_UPDATE_SHA256", &staged.sha256)
        .env("RKN_UPDATE_SIZE", update.size.to_string()))?;
    Ok(staged)
}

pub fn take_error(executable: &Path) -> Option<String> {
    let name = executable
        .file_name()
        .unwrap_or(OsStr::new("rkn-guard-panel.exe"));
    let mut error_name = name.to_os_string();
    error_name.push(".update-error");
    let path = executable.with_file_name(error_name);
    let error = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(path);
    Some(error)
}

enum State {
    Checking(Receiver<Result<Check>>),
    Checked(Check),
    Downloading(Receiver<Result<Staged>>),
    Ready(Staged),
    Failed(String),
    Restart,
}

pub struct Updater(State);

impl Updater {
    pub fn start() -> Self {
        if let Some(error) = std::env::current_exe()
            .ok()
            .and_then(|path| take_error(&path))
        {
            return Self(State::Failed(error));
        }
        Self(State::Checking(background(check)))
    }

    pub fn show(&mut self, ui: &mut egui::Ui, busy: bool) {
        self.poll(ui.ctx(), busy);
        ui.label(egui::RichText::new(format!("Панель {VERSION}")).strong());
        let (status, button) = match &self.0 {
            State::Checking(_) => ("Проверяем GitHub…".into(), None),
            State::Downloading(_) => ("Скачиваем и проверяем EXE…".into(), None),
            State::Ready(_) => ("Ждём завершения операции на сервере…".into(), None),
            State::Restart => ("Перезапускаем панель…".into(), None),
            State::Checked(Check::Current) => ("Актуальная версия".into(), Some("Проверить")),
            State::Checked(Check::Missing) => ("Релиз пока недоступен".into(), Some("Проверить")),
            State::Checked(Check::NoAsset(tag)) => {
                (format!("{tag} без EXE или SHA-256"), Some("Проверить"))
            }
            State::Checked(Check::Available(update)) => {
                (format!("Доступна {}", update.tag), Some("Обновить"))
            }
            State::Failed(error) => {
                ui.label(egui::RichText::new("Ошибка обновления").color(egui::Color32::LIGHT_RED))
                    .on_hover_text(error);
                ("".into(), Some("Повторить"))
            }
        };
        if !status.is_empty() {
            ui.label(status);
        }
        if let Some(label) = button {
            let blocked = busy && matches!(self.0, State::Checked(Check::Available(_)));
            if ui
                .add_enabled(!blocked, egui::Button::new(label))
                .on_disabled_hover_text("Дождитесь завершения операции на сервере")
                .clicked()
            {
                self.0 = match std::mem::replace(&mut self.0, State::Restart) {
                    State::Checked(Check::Available(update)) => {
                        State::Downloading(background(move || download(update)))
                    }
                    _ => State::Checking(background(check)),
                };
            }
        } else if !matches!(self.0, State::Restart) {
            ui.spinner();
        }
    }

    fn poll(&mut self, ctx: &egui::Context, busy: bool) {
        match &self.0 {
            State::Checking(receiver) => {
                if let Some(result) = received(receiver) {
                    self.0 = match result {
                        Ok(check) => State::Checked(check),
                        Err(err) => State::Failed(format!("{err:#}")),
                    };
                }
            }
            State::Downloading(receiver) => {
                if let Some(result) = received(receiver) {
                    self.0 = match result {
                        Ok(staged) => State::Ready(staged),
                        Err(err) => State::Failed(format!("{err:#}")),
                    };
                }
            }
            State::Ready(_) if !busy => {
                if let State::Ready(staged) = std::mem::replace(&mut self.0, State::Restart) {
                    match staged.launch() {
                        Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                        Err(err) => self.0 = State::Failed(format!("{err:#}")),
                    }
                }
            }
            _ => {}
        }
    }
}

fn background<T: Send + 'static>(
    task: impl FnOnce() -> Result<T> + Send + 'static,
) -> Receiver<Result<T>> {
    let (send, receive) = mpsc::channel();
    thread::spawn(move || {
        let _ = send.send(task());
    });
    receive
}

fn received<T>(receiver: &Receiver<Result<T>>) -> Option<Result<T>> {
    match receiver.try_recv() {
        Ok(result) => Some(result),
        Err(TryRecvError::Empty) => None,
        Err(TryRecvError::Disconnected) => {
            Some(Err(anyhow::anyhow!("Проверка обновлений прервалась")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn сверка_версии_и_файла_релиза() -> Result<()> {
        assert_eq!(version("v1.10.0"), Some([1, 10, 0]));
        assert!(version("v1.0.0-beta").is_none());
        assert!(version("v1.0.0/../../file").is_none());
        assert!(matches!(
            evaluate(Release {
                tag_name: VERSION.into(),
                assets: vec![]
            })?,
            Check::Current
        ));
        assert!(matches!(
            evaluate(Release {
                tag_name: "v99.0.0".into(),
                assets: vec![]
            })?,
            Check::NoAsset(_)
        ));
        let asset = Asset {
            name: "rkn-guard-panel.exe".into(),
            digest: Some(format!("sha256:{}", "a".repeat(64))),
            size: 500,
        };
        assert!(matches!(
            evaluate(Release {
                tag_name: "v99.0.0".into(),
                assets: vec![asset]
            })?,
            Check::Available(_)
        ));
        Ok(())
    }
}
