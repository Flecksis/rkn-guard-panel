use super::*;
use russh::{
    Channel, ChannelId,
    server::{self, Server as _},
};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
#[derive(Clone)]
struct Node {
    credentials: Arc<Mutex<(Option<PublicKey>, Option<PublicKey>)>>,
    connections: Arc<AtomicUsize>,
}
impl server::Server for Node {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.connections.fetch_add(1, Ordering::SeqCst);
        self.clone()
    }
}
impl server::Handler for Node {
    type Error = russh::Error;
    async fn auth_publickey(
        &mut self,
        user: &str,
        key: server::VerifiedPublicKey<'_>,
    ) -> Result<server::Auth, Self::Error> {
        let keys = self.credentials.lock().unwrap();
        let matches = |k: &Option<PublicKey>| {
            k.as_ref()
                .is_some_and(|k| k.key_data() == key.public_key.key_data())
        };
        Ok(
            if user == "rkn-guard-user" && (matches(&keys.0) || matches(&keys.1)) {
                server::Auth::Accept
            } else {
                server::Auth::reject()
            },
        )
    }
    async fn channel_open_session(
        &mut self,
        _: Channel<server::Msg>,
        reply: server::ChannelOpenHandle,
        _: &mut server::Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }
    async fn exec_request(
        &mut self,
        ch: ChannelId,
        cmd: &[u8],
        s: &mut server::Session,
    ) -> Result<(), Self::Error> {
        if matches!(cmd, b"panel enroll" | b"panel session") {
            s.channel_success(ch)?;
        } else {
            s.channel_failure(ch)?;
        }
        Ok(())
    }
    async fn data(
        &mut self,
        ch: ChannelId,
        data: &[u8],
        s: &mut server::Session,
    ) -> Result<(), Self::Error> {
        let input: Value = serde_json::from_slice(data).unwrap();
        let response = if let Some(pk) = input["public_key"].as_str() {
            let mut k = self.credentials.lock().unwrap();
            assert!(k.0.take().is_some());
            k.1 = Some(PublicKey::from_openssh(pk).unwrap());
            json!({"ok":true})
        } else if input["op"] == "update" {
            s.extended_data(ch, 1, "RKN_STAGE:bu")?;
            s.extended_data(ch, 1, "ild\nRKN_STAGE:heartbeat\n")?;
            json!({"ok":true,"data":{}})
        } else {
            json!({"ok":true,"data":{"cursor":"1","boot_id":"test","version":"test","firewall":"nftables","events":[],"families":[{"family":"4","packets":7,"bytes":900}]}})
        };
        s.data(ch, serde_json::to_vec(&response).unwrap())?;
        s.eof(ch)?;
        s.close(ch)?;
        Ok(())
    }
}
#[test]
fn enroll_restart_and_reuse_one_connection() -> Result<()> {
    let host_key = PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)?;
    let bootstrap = PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519)?;
    let host_pub = host_key.public_key().to_openssh()?;
    let node = Node {
        credentials: Arc::new(Mutex::new((Some(bootstrap.public_key().clone()), None))),
        connections: Arc::new(AtomicUsize::new(0)),
    };
    let count = node.connections.clone();
    let socket = runtime().block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))?;
    let port = socket.local_addr()?.port();
    let listener = runtime().spawn(async move {
        let mut node = node;
        let cfg = server::Config {
            keys: vec![host_key],
            ..Default::default()
        };
        node.run_on_socket(Arc::new(cfg), &socket).await.unwrap()
    });
    let data = json!({"host":"127.0.0.1","port":port,"host_key":host_pub,"bootstrap_key":bootstrap.to_openssh(Default::default())?.as_str(),"expires":chrono::Utc::now().timestamp()+60});
    let code = format!(
        "RKN1.{}",
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(&data)?)
    );
    let tmp = tempfile::tempdir()?;
    let path = tmp.path().join("test.sqlite3");
    pair(&path, "Node".into(), code.clone())?;
    let mut db = crate::db::open(&path)?;
    let host = crate::db::hosts(&db)?.remove(0);
    assert!(host.active);
    let mut ssh = Session::start(&host, &protect(&host.secret, false)?, "panel session")?;
    let mut stages = Vec::new();
    ssh.request_with_progress(json!({"op":"update"}), |stage| {
        stages.push(stage.to_owned())
    })?;
    assert_eq!(stages, ["build", "heartbeat"]);
    for _ in 0..2 {
        let reading = ssh.request(json!({"op":"snapshot"}))?;
        crate::db::save(&mut db, &host, serde_json::from_value(reading)?)?;
    }
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM samples", [], |r| r.get::<_, i64>(0))?,
        2
    );
    // Имитируем потерю ответа после установки ключа на сервере.
    db.execute("UPDATE panel_hosts SET active=0", [])?;
    pair(&path, "Node".into(), code)?;
    let recovered = crate::db::hosts(&db)?;
    assert_eq!(recovered.len(), 1);
    assert!(recovered[0].active);
    assert_eq!(recovered[0].secret, host.secret);
    listener.abort();
    Ok(())
}
