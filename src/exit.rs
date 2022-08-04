use super::code::{decode, encode};
use super::error::ExitNodeError;
use rand::Rng;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Config {
    pub target_host: String,
    pub target_port: u16,
}

struct ExitSession {
    pub id: u128,
    pub is_closed: Arc<tokio::sync::Mutex<bool>>,
    pub tcp_stream: Arc<tokio::sync::Mutex<tokio::net::TcpStream>>,
}

impl ExitSession {
    pub async fn close(&self) -> Result<(), ExitNodeError> {
        let mut guard = self.tcp_stream.lock().await;
        guard.shutdown().await?;
        Ok(())
    }

    pub async fn send(&self, buf: Vec<u8>) -> Result<(), ExitNodeError> {
        let mut buf = buf;
        let mut guard = self.tcp_stream.lock().await;
        guard.write_all(&mut buf).await?;
        Ok(())
    }

    pub async fn send_and_receive(&self, buf: Vec<u8>) -> Result<Vec<u8>, ExitNodeError> {
        let bytes_to_send = buf.len();
        // Send first
        if bytes_to_send > 0 {
            let mut buf = buf;
            let mut guard = self.tcp_stream.lock().await;
            guard.write_all(&mut buf).await?;
        }
        // Read
        let mut response_buf = vec![0u8; 1048576]; // Allocate 1MB
        let bytes_received = if bytes_to_send == 0 {
            // There were no bytes to send. If there is nothing to read, wait.
            let mut break_loop = true;
            let mut bytes_read_count = 0;
            while break_loop {
                let read_trial = {
                    let guard = self.tcp_stream.lock().await;
                    guard.try_read(&mut response_buf)
                };
                bytes_read_count = match read_trial {
                    Ok(0) => {
                        break_loop = false;
                        Err(ExitNodeError::closed())
                    }
                    Ok(n) => {
                        break_loop = false;
                        Ok(n)
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        tokio::task::yield_now().await;
                        Ok(0)
                    }
                    Err(e) => Err(e)?,
                }?;
                {
                    let close_guard = self.is_closed.lock().await;
                    if *close_guard {
                        println!("Session has been closed by the EntryPoint via HTTP.");
                        return Err(ExitNodeError::closed());
                    }
                }
            }
            bytes_read_count
        } else {
            // There were bytes to send. If there are no bytes to read, just return.
            let guard = self.tcp_stream.lock().await;
            match guard.try_read(&mut response_buf) {
                Ok(n) => Ok(n),
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(0),
                Err(e) => Err(e),
            }?
        };
        response_buf.truncate(bytes_received);
        Ok(response_buf)
    }
}

struct ExitSessionManager {
    target_string: String,
    // sessions: Arc<Mutex<HashMap<u128, Arc<tokio::sync::Mutex<ExitSession>>>>>,
    sessions: Arc<tokio::sync::RwLock<HashMap<u128, ExitSession>>>,
}

// #[async_trait]
impl ExitSessionManager {
    pub fn new(target_string: String) -> Self {
        Self {
            target_string,
            sessions: Arc::new(tokio::sync::RwLock::new(HashMap::new())),
            // sessions: HashMap::new(),
        }
    }

    async fn new_session(&self) -> Result<u128, ExitNodeError> {
        let stream = tokio::net::TcpStream::connect(&self.target_string).await?;
        let uid: u128 = {
            let mut rng = rand::thread_rng();
            rng.gen()
        };
        let mut guard = self.sessions.write().await;
        guard.insert(
            uid,
            // Arc::new(tokio::sync::Mutex::new(ExitSession {
            ExitSession {
                id: uid,
                is_closed: Arc::new(tokio::sync::Mutex::new(false)),
                tcp_stream: Arc::new(tokio::sync::Mutex::new(stream)),
            },
        );
        Ok(uid)
    }

    async fn close(&self, uid: u128) -> Result<(), ExitNodeError> {
        {
            let guard = self.sessions.read().await;
            let session = guard.get(&uid).ok_or(ExitNodeError::f(format!(
                "Session with ID {} not found",
                uid
            )))?;
            {
                let mut guard2 = session.is_closed.lock().await;
                *guard2 = true;
            }
        }
        let mut guard = self.sessions.write().await;
        guard.remove(&uid).expect("Please never happen #2");

        Ok(())
    }

    async fn send_and_receive(&self, uid: u128, buf: Vec<u8>) -> Result<Vec<u8>, ExitNodeError> {
        // Send first
        let guard = self.sessions.read().await;
        let sess = guard
            .get(&uid)
            .ok_or(ExitNodeError::f(format!("Session {} not found!", uid)))?;

        // let mut guard = sess.lock().await;
        sess.send_and_receive(buf).await
    }

    async fn send(&self, uid: u128, buf: Vec<u8>) -> Result<(), ExitNodeError> {
        let guard = self.sessions.read().await;
        let sess = guard
            .get(&uid)
            .ok_or(ExitNodeError::f(format!("Session {} not found!", uid)))?;
        sess.send(buf).await
    }
}

#[post("/open")]
async fn open(manager: &rocket::State<ExitSessionManager>) -> Result<String, ExitNodeError> {
    let uid = manager.new_session().await?;
    Ok(uid.to_string())
}

#[post("/u/<uid>", data = "<data>")]
async fn upload(
    manager: &rocket::State<ExitSessionManager>,
    uid: u128,
    data: String,
) -> Result<String, ExitNodeError> {
    let http_receive_data = decode(data)?;
    let http_receive_bytes = http_receive_data.len();
    print!(".");
    manager.send(uid, http_receive_data).await?;
    Ok("".to_string())
}

#[post("/s/<uid>", data = "<data>")]
async fn sync(
    manager: &rocket::State<ExitSessionManager>,
    uid: u128,
    data: Option<String>,
) -> Result<String, ExitNodeError> {
    let http_receive_data = decode(data.unwrap_or("".to_string()))?;
    let http_receive_bytes = http_receive_data.len();
    let response_data = manager.send_and_receive(uid, http_receive_data).await?;
    let http_response_data = encode(response_data);
    print!("-");
    // println!(
    //     "HTTP Received bytes {}, HTTP Upload bytes {}",
    //     http_receive_bytes,
    //     http_response_data.len()
    // );
    Ok(http_response_data)
}

#[post("/close/<uid>")]
async fn close(
    manager: &rocket::State<ExitSessionManager>,
    uid: u128,
) -> Result<String, ExitNodeError> {
    println!("Close session {}", uid);
    manager.close(uid).await?;
    Ok("".to_string())
}

pub async fn exit_main(
    bind_ip: std::net::IpAddr,
    listen_http_port: u16,
    target_host: String,
    target_port: u16,
    debug: bool,
) {
    let default_config = if debug {
        rocket::Config::debug_default()
    } else {
        rocket::Config::default()
    };
    let rocket_config = rocket::Config {
        port: listen_http_port,
        address: bind_ip,
        ..default_config
    };
    let session_manager = ExitSessionManager::new(format!("{}:{}", target_host, target_port));
    let config = Config {
        target_host,
        target_port,
    };
    let _r = rocket::custom(&rocket_config)
        .mount("/", routes![open, sync, upload, close])
        .manage(session_manager)
        .manage(config)
        .ignite()
        .await
        .expect("Nope")
        .launch()
        .await
        .expect("Nope nope");
}
