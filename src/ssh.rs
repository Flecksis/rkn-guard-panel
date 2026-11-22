use crate::db::Host;
#[cfg(test)]
mod tests;
use anyhow::{Context, Result, bail, ensure};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use russh::{
    ChannelMsg, client,
    keys::{PrivateKey, PrivateKeyWithHashAlg, PublicKey, PublicKeyOrCertificate},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, OnceLock},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData, CryptUnprotectData},
};

pub fn protect(data: &[u8], encrypt: bool) -> Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len().try_into()?,
        pbData: data.as_ptr() as _,
    };
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    unsafe {
        let ok = if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                1,
                &mut out,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                1,
                &mut out,
            )
        };
        ensure!(
            ok != 0,
            "Windows DPAPI: {}",
            std::io::Error::last_os_error()
        );
        let result = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        Ok(result)
    }
}
#[derive(Deserialize)]
pub struct Pair {
    host: String,
    port: u16,
    host_key: String,
    bootstrap_key: String,
    // Срок проверяет сервер по своим часам при входе и выдаче постоянного ключа.
    #[serde(rename = "expires")]
    _expires: i64,
}
fn decode(code: &str) -> Result<Pair> {
    ensure!(code.len() < 32768, "Слишком длинный код");
    let p: Pair = serde_json::from_slice(
        &URL_SAFE_NO_PAD.decode(
            code.trim()
                .strip_prefix("RKN1.")
                .context("Ожидается код RKN1")?,
        )?,
    )?;
    ensure!(
        p.port != 0
            && !p.host.is_empty()
            && p.host.len() < 254
            && !p.host.starts_with('-')
            && p.host
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b".:-".contains(&c)),
        "Некорректный адрес"
    );
    let (kind, key) = p.host_key.split_once(' ').context("Нет ключа сервера")?;
    ensure!(
        matches!(kind, "ssh-ed25519" | "ssh-rsa" | "ecdsa-sha2-nistp256")
            && STANDARD.decode(key).is_ok(),
        "Некорректный ключ сервера"
    );
    ensure!(
        p.bootstrap_key
            .starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"),
        "Нет временного SSH-ключа"
    );
    Ok(p)
}
struct Peer(PublicKey);
impl client::Handler for Peer {
    type Error = russh::Error;
    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> std::result::Result<bool, Self::Error> {
        Ok(key.certificate().is_none() && key.public_key().key_data() == self.0.key_data())
    }
}
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("SSH runtime")
    })
}
pub struct Session {
    conn: client::Handle<Peer>,
    operation: String,
}
impl Session {
    pub fn start(h: &Host, key: &[u8], operation: &str) -> Result<Self> {
        let identity = PrivateKey::from_openssh(key)?;
        let peer = Peer(PublicKey::from_openssh(&h.host_key)?);
        let cfg = client::Config {
            keepalive_interval: Some(Duration::from_secs(15)),
            ..Default::default()
        };
        let conn = runtime().block_on(async {
            let limit = Duration::from_secs(20);
            let addr = (h.addr.as_str(), h.port);
            let stream = tokio::time::timeout(limit, tokio::net::TcpStream::connect(addr))
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "Нет ответа от {} (порт {}). Проверьте доступность SSH с этого компьютера.",
                        h.addr,
                        h.port
                    )
                })?
                .with_context(|| {
                    format!(
                        "Не удалось открыть TCP-соединение с {} (порт {})",
                        h.addr, h.port
                    )
                })?;
            let mut conn = tokio::time::timeout(
                limit,
                client::connect_stream(Arc::new(cfg), stream, peer),
            )
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "TCP-соединение установлено, но сервер не завершил SSH-рукопожатие за 20 секунд"
                )
            })?
            .context("Ошибка SSH-рукопожатия или проверки ключа хоста")?;
            let auth = tokio::time::timeout(
                limit,
                conn.authenticate_publickey(
                    "rkn-guard-user",
                    PrivateKeyWithHashAlg::new(Arc::new(identity), None),
                ),
            )
            .await
            .map_err(|_| {
                anyhow::anyhow!("Сервер не ответил на запрос входа rkn-guard-user за 20 секунд")
            })??;
            if !auth.success() && operation == "panel enroll" {
                bail!("Сервер отклонил временный ключ. Получите новый код в пункте 9 меню rkn и повторите подключение.");
            }
            ensure!(auth.success(), "Сервер отклонил ключ панели");
            Ok::<_, anyhow::Error>(conn)
        })?;
        Ok(Self {
            conn,
            operation: operation.into(),
        })
    }
    pub fn request(&mut self, v: Value) -> Result<Value> {
        self.request_with_progress(v, |_| {})
    }
    pub fn request_with_progress(
        &mut self,
        v: Value,
        mut stage: impl FnMut(&str),
    ) -> Result<Value> {
        let limit = if matches!(v["op"].as_str(), Some("update" | "manage")) {
            900
        } else {
            90
        };
        let mut data = serde_json::to_vec(&v)?;
        data.push(b'\n');
        let response = runtime().block_on(async {
            tokio::time::timeout(Duration::from_secs(limit), async {
                let mut ch = self.conn.channel_open_session().await?;
                ch.exec(true, self.operation.as_bytes()).await?;
                ch.data(&data[..]).await?;
                ch.eof().await?;
                let mut raw = Vec::new();
                let mut progress = Vec::new();
                while let Some(msg) = ch.wait().await {
                    match msg {
                        ChannelMsg::Data { data } => {
                            ensure!(
                                raw.len() + data.len() <= 4 * 1024 * 1024,
                                "Превышен размер ответа"
                            );
                            raw.extend_from_slice(&data);
                        }
                        ChannelMsg::ExtendedData { data, ext: 1 } => {
                            for byte in data.iter().copied() {
                                if byte == b'\n' {
                                    if let Ok(line) = std::str::from_utf8(&progress) {
                                        if let Some(code) = line.strip_prefix("RKN_STAGE:") {
                                            stage(code.trim_end());
                                        }
                                    }
                                    progress.clear();
                                } else if progress.len() < 256 {
                                    progress.push(byte);
                                }
                            }
                        }
                        ChannelMsg::ExitStatus { exit_status } if exit_status != 0 => {
                            bail!("Сервер вернул код {exit_status}")
                        }
                        ChannelMsg::Eof | ChannelMsg::Close => break,
                        _ => {}
                    }
                }
                Ok::<Value, anyhow::Error>(
                    serde_json::from_slice(&raw).context("Неверный ответ CLI RKNGuard")?,
                )
            })
            .await
            .map_err(|_| {
                anyhow::anyhow!("Команда {} не завершилась за {limit} сек.", self.operation,)
            })?
        })?;
        if let Some(e) = response["error"].as_str() {
            bail!("{}", e)
        }
        ensure!(response["ok"] == true, "Операция не подтверждена сервером");
        Ok(response["data"].clone())
    }
}
pub fn pair(dbpath: &Path, name: String, code: String) -> Result<String> {
    let name = name.trim().to_owned();
    ensure!(
        !name.is_empty() && name.chars().count() <= 80 && !name.chars().any(char::is_control),
        "Укажи имя сервера"
    );
    let p = decode(&code)?;
    let db = crate::db::open(dbpath)?;
    let mut existing = None;
    for h in crate::db::hosts(&db)? {
        if h.name == name || (h.addr == p.host && h.port == p.port) {
            ensure!(
                !h.active
                    && h.name == name
                    && h.addr == p.host
                    && h.port == p.port
                    && h.host_key == p.host_key,
                "Сервер уже добавлен"
            );
            existing = Some(h);
        }
    }
    let retry = existing.is_some();
    let (host, identity) = if let Some(host) = existing {
        let key = protect(&host.secret, false)?;
        // Сервер мог принять код, даже если ответ потерялся.
        if Session::start(&host, &key, "panel session").is_ok() {
            db.execute("UPDATE panel_hosts SET active=1 WHERE name=?", [&name])?;
            return Ok(name);
        }
        (host, PrivateKey::from_openssh(&key)?)
    } else {
        let identity = PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)?;
        let host = Host {
            name: name.clone(),
            addr: p.host,
            port: p.port,
            host_key: p.host_key,
            secret: protect(identity.to_openssh(Default::default())?.as_bytes(), true)?,
            active: false,
        };
        (host, identity)
    };
    let public = identity.public_key().to_openssh()?;
    let mut sess = Session::start(&host, p.bootstrap_key.as_bytes(), "panel enroll")?;
    if !retry {
        db.execute(
            "INSERT INTO panel_hosts VALUES(?,?,?,?,?,0)",
            rusqlite::params![host.name, host.addr, host.port, host.host_key, host.secret],
        )?;
    }
    sess.request(json!({"public_key":public.trim()}))?;
    db.execute("UPDATE panel_hosts SET active=1 WHERE name=?", [&name])?;
    Ok(name)
}
