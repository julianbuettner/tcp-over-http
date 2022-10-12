use crate::artex;
use crate::{ouroboros_impl_wrapper::WrapperBuilder, Artex};
use actix_web::dev::Server;
use actix_web::{get, post, web, App, HttpResponse, HttpServer, Responder};
use anyhow::anyhow;
use futures::stream::TryStreamExt;
use halfbrown::HashMap as Map;
use std::net::SocketAddr;
use stream_cancel::{Trigger, Valve};
use tokio::{net::TcpStream, sync::RwLock};
use tokio_util::codec::{BytesCodec, FramedRead};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Debug)]
pub(crate) struct UpExitSession {
    pub(crate) tcp_out: Artex<tokio::net::tcp::OwnedWriteHalf>,
    pub(crate) stop_copy: CancellationToken,
}

use derivative::Derivative;

#[derive(Derivative)]
#[derivative(Debug)]
pub(crate) struct DownExitSession {
    pub(crate) tcp_in: Artex<tokio::net::tcp::OwnedReadHalf>,
    #[derivative(Debug = "ignore")]
    stream_valve: Valve,
    #[derivative(Debug = "ignore")]
    stop_stream: Trigger,
}

#[derive(Debug)]
pub(crate) struct ExitSession {
    pub(crate) up: UpExitSession,
    pub(crate) down: DownExitSession,
}
impl ExitSession {
    fn new(conn: TcpStream) -> Self {
        let (down, up) = conn.into_split();
        let (trigger, valve) = Valve::new();
        ExitSession {
            up: UpExitSession {
                tcp_out: artex(up),
                stop_copy: CancellationToken::new(),
            },
            down: DownExitSession {
                tcp_in: artex(down),
                stream_valve: valve,
                stop_stream: trigger,
            },
        }
    }
}

#[derive(Debug)]
pub(crate) struct ExitSessionManager {
    target_addr: Vec<SocketAddr>,
    pub(crate) sessions: RwLock<Map<Uuid, ExitSession>>,
}

impl ExitSessionManager {
    fn new(target_addr: Vec<SocketAddr>) -> Self {
        Self {
            target_addr,
            sessions: tokio::sync::RwLock::new(Map::new()),
        }
    }
}

#[get("/open")]
async fn open(manager: web::Data<ExitSessionManager>) -> Vec<u8> {
    let stream = match TcpStream::connect(manager.target_addr.as_slice()).await {
        Ok(x) => x,
        Err(x) => {
            dbg!(x, "couldnt connect to target");
            //signal
            return vec![];
        }
    };
    let uid = Uuid::new_v4();
    let mut guard = manager.sessions.write().await;
    guard.insert(uid, ExitSession::new(stream));
    return uid.into_bytes().to_vec();
}

#[post("/upload/{uid_s}")]
async fn upload(
    manager: web::Data<ExitSessionManager>,
    uid_s: web::Path<String>,
    http_receive_data: web::Payload,
) -> impl Responder {
    let (guard, stop_copy) = {
        let uid = Uuid::parse_str(&uid_s).unwrap();
        let guard = manager.sessions.read().await;
        let up_sess = &guard
            .get(&uid)
            .ok_or_else(|| anyhow!("Session {:#x?} not found!", uid))
            .unwrap()
            .up;
        (
            up_sess.tcp_out.clone().lock_owned(),
            up_sess.stop_copy.clone(),
        )
    };
    let r = http_receive_data
        .map_err(|x| Err(x).unwrap())
        .into_async_read();
    let mut r = tokio_util::compat::FuturesAsyncReadCompatExt::compat(r);
    let tcp_out = &mut *guard.await;
    return tokio::select! {
        x = tokio::io::copy(&mut r, tcp_out) => {
            if let Err(x) = x {
                dbg!("target disconnect", x);
                HttpResponse::Ok().body("target disconnect")
            } else {
                HttpResponse::Ok().body("finished")
            }
        }
        _ = stop_copy.cancelled() => {
            HttpResponse::Ok().body("cancelled")
        }
    };
}

#[get("/download/{uid_s}")]
async fn download(
    manager: web::Data<ExitSessionManager>,
    uid_s: web::Path<String>,
) -> impl Responder {
    let (guard, valve) = {
        let uid = Uuid::parse_str(&uid_s).unwrap();
        let guard = manager.sessions.read().await;
        let down_sess = &guard
            .get(&uid)
            .ok_or_else(|| anyhow!("Session {:#x?} not found!", uid))
            .unwrap()
            .down;
        (
            down_sess.tcp_in.clone().lock_owned(),
            down_sess.stream_valve.clone(),
        )
    };
    let stream = WrapperBuilder {
        guard: guard.await,
        fr_builder: |a| FramedRead::new(a, BytesCodec::new()),
    }
    .build();
    let stream = valve.wrap(stream);
    //BytesMut -> Bytes
    return HttpResponse::Ok().streaming(stream.map_ok(Into::into));
}

#[get("/close/{uid_s}")]
async fn close(manager: web::Data<ExitSessionManager>, uid_s: web::Path<String>) -> impl Responder {
    let uid = Uuid::parse_str(&uid_s).unwrap();
    println!("Close session {uid:#x?}");
    let mut guard = manager.sessions.write().await;
    let sess = guard
        .remove(&uid)
        .ok_or_else(|| anyhow!("Session with ID {uid:#x?} not found"))
        .unwrap();
    sess.down.stop_stream.cancel();
    sess.up.stop_copy.cancel();
    HttpResponse::Ok()
}

pub fn main(bind_addr: &[SocketAddr], target_addr: Vec<SocketAddr>) -> (Vec<SocketAddr>, Server) {
    let session_manager = web::Data::new(ExitSessionManager::new(target_addr));
    #[cfg(test)]
    {
        *test::ARC.try_lock().unwrap() = Some(session_manager.clone());
    }
    let x = HttpServer::new(move || {
        App::new()
            .app_data(session_manager.clone())
            //.app_data(web::PayloadConfig::new(1024 * 1024))
            .service(open)
            .service(upload)
            .service(download)
            .service(close)
    })
    .bind(bind_addr)
    .unwrap();
    let bound = x.addrs();
    println!("Listening on {bound:?}");
    return (bound, x.run());
}

#[cfg(test)]
pub(crate) mod test {
    use super::ExitSessionManager;
    use actix_web::web;
    use tokio::sync::Mutex;

    pub(crate) static ARC: Mutex<Option<web::Data<ExitSessionManager>>> = Mutex::const_new(None);
}
