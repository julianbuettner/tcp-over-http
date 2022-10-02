use crate::artex;
use crate::{ouroboros_impl_wrapper::WrapperBuilder, Artex};
use actix_web::{get, post, web, App, HttpResponse, HttpServer, Responder};
use anyhow::anyhow;
use futures::stream::TryStreamExt;
use halfbrown::HashMap as Map;
use std::net::SocketAddr;
use stream_cancel::{Trigger, Valve};
use tokio::select;
use tokio::{net::TcpStream, sync::RwLock};
use tokio_util::codec::{BytesCodec, FramedRead};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

type UpExitSession = tokio::net::tcp::OwnedWriteHalf;
type DownExitSession = tokio::net::tcp::OwnedReadHalf;

struct ExitSessionManager {
    target_addr: Vec<SocketAddr>,
    #[allow(clippy::type_complexity)]
    sessions: RwLock<
        Map<
            Uuid,
            (
                (Artex<UpExitSession>, CancellationToken),
                (Artex<DownExitSession>, (Trigger, Valve)),
            ),
        >,
    >,
}

// #[async_trait]
impl ExitSessionManager {
    pub fn new(target_addr: Vec<SocketAddr>) -> Self {
        Self {
            target_addr,
            sessions: tokio::sync::RwLock::new(Map::new()),
        }
    }

    async fn new_session(&self) -> Uuid {
        //dbg!(&self.target_addr);
        let stream = TcpStream::connect(self.target_addr.as_slice())
            .await
            .unwrap();
        let uid = Uuid::new_v4();
        let mut guard = self.sessions.write().await;
        let (down, up) = stream.into_split();
        guard.insert(
            uid,
            (
                (artex(up), CancellationToken::new()),
                (artex(down), Valve::new()),
            ),
        );
        return uid;
    }

    async fn close(&self, uid: Uuid) {
        let mut guard = self.sessions.write().await;
        let (up, down) = guard
            .remove(&uid)
            .ok_or_else(|| anyhow!("Session with ID {:#x?} not found", uid))
            .unwrap();
        up.1.cancel();
        down.1 .0.cancel();
    }
}

#[get("/open")]
async fn open(manager: web::Data<ExitSessionManager>) -> Vec<u8> {
    let uid = manager.new_session().await;
    uid.into_bytes().to_vec()
}

#[post("/upload/{uid_s}")]
async fn upload(
    manager: web::Data<ExitSessionManager>,
    uid_s: web::Path<String>,
    http_receive_data: web::Payload,
) -> impl Responder {
    let (guard, token) = {
        let uid = Uuid::parse_str(&uid_s).unwrap();
        let guard = manager.sessions.read().await;
        let (up, _) = guard
            .get(&uid)
            .ok_or_else(|| anyhow!("Session {:#x?} not found!", uid))
            .unwrap();
        (up.0.clone().lock_owned(), up.1.clone())
    };
    let r = http_receive_data
        .map_err(|x| Err(x).unwrap())
        .into_async_read();
    let mut r = tokio_util::compat::FuturesAsyncReadCompatExt::compat(r);
    let cf = async {
        let mut guard = guard.await;
        tokio::io::copy(&mut r, &mut *guard).await
    };
    return select! {
        x = cf => {
            x.unwrap();
            HttpResponse::Ok()
        },
        _ = token.cancelled() => {
            //todo should err?
            //actix_web::HttpResponseBuilder::new(StatusCode::not_ok)
            HttpResponse::Ok()
        }
    };
}

#[get("/download/{uid_s}")]
async fn download(
    manager: web::Data<ExitSessionManager>,
    uid_s: web::Path<String>,
) -> impl Responder {
    /*let http_response_data = manager
        .receive(Uuid::parse_str(&uid_s).unwrap())
        .await
        .unwrap();
    http_response_data*/
    let stream = {
        let uid = Uuid::parse_str(&uid_s).unwrap();
        let guard = manager.sessions.read().await;
        let (_, down) = guard
            .get(&uid)
            .ok_or_else(|| anyhow!("Session {:#x?} not found!", uid))
            .unwrap();
        let valve = &down.1 .1;
        let guard = down.0.clone().lock_owned().await;
        let stream = WrapperBuilder {
            guard,
            fr_builder: |a| FramedRead::new(a, BytesCodec::new()),
        }
        .build();
        valve.wrap(stream)
    };
    //BytesMut -> Bytes
    return HttpResponse::Ok().streaming(stream.map_ok(Into::into));
}

struct Test;
impl Drop for Test {
    fn drop(&mut self) {
        panic!()
    }
}

#[get("/close/{uid_s}")]
async fn close(manager: web::Data<ExitSessionManager>, uid_s: web::Path<String>) -> impl Responder {
    let uid = Uuid::parse_str(&uid_s).unwrap();
    println!("Close session {:#x?}", uid);
    manager.close(uid).await;
    HttpResponse::Ok()
}

pub async fn main(bind_addr: &[SocketAddr], target_addr: Vec<SocketAddr>) {
    let session_manager = web::Data::new(ExitSessionManager::new(target_addr));
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
    println!("Listening on {:?}", x.addrs());
    x.run().await.unwrap();
    /*let _r = rocket::custom(&rocket_config)
    .mount(
        "/",
        #[allow(clippy::no_effect_underscore_binding)]
        {
            routes![open, sync, upload, close]
        },
    )
    .manage(session_manager)
    .manage(Config)
    .ignite()
    .await
    .expect("Nope")
    .launch()
    .await
    .expect("Nope nope");*/
}
