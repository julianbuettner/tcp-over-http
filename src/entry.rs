#![allow(clippy::items_after_statements)]

use crate::ouroboros_impl_wrapper::WrapperBuilder;
use crate::{artex, join_url};

use bytes::Bytes;
use futures::TryStreamExt;
use reqwest::{Body, Client, Response, Url};
use std::convert::{identity, TryInto};
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_util::codec::{BytesCodec, FramedRead};
use tokio_util::compat::FuturesAsyncReadCompatExt;
use uuid::Uuid;
//use tokio::time::timeout;

lazy_static::lazy_static! {
    pub(crate) static ref CLIENT: Client = Client::new();
}

async fn close_session(target: &Url, uid: Uuid) {
    let resp = CLIENT
        .get(join_url(target, ["close/", &uid.to_string()]))
        .send()
        .await
        .unwrap();
    assert_ok(resp).await;
}
async fn init_http_session(target: &Url) -> Uuid {
    let resp = CLIENT.get(join_url(target, ["open"])).send().await.unwrap();
    let resp = assert_ok(resp).await;
    /*use futures::stream::TryStreamExt;
    let s = resp.bytes_stream();
    let r = s.map_err(|x| Err(x).unwrap()).into_async_read();
    let mut r = tokio_util::compat::FuturesAsyncReadCompatExt::compat(r);
    let mut uid = Uid::zero();
    r.read_exact(uid.get_mut()).await.unwrap();
    return uid;*/
    return Uuid::from_bytes(
        identity::<&[u8]>(&resp.bytes().await.unwrap())
            .try_into()
            .unwrap(),
    );
}

async fn upload_data<S>(target: &Url, uid: Uuid, data: S)
where
    S: futures::TryStream + Send + Sync + 'static,
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    bytes::Bytes: From<S::Ok>,
{
    let resp = CLIENT
        .post(join_url(target, ["upload/", &uid.to_string()]))
        .body(Body::wrap_stream(data))
        .send()
        .await
        .unwrap();
    assert_ok(resp).await;
}

async fn download_data(
    target: &Url,
    uid: Uuid,
) -> impl futures::Stream<Item = reqwest::Result<Bytes>> {
    let resp = CLIENT
        .get(join_url(target, ["download/", &uid.to_string()]))
        .send()
        .await
        .unwrap();
    let resp = assert_ok(resp).await;
    return resp.bytes_stream();
}

async fn process_socket(target_url: Arc<Url>, socket: tokio::net::TcpStream) -> Uuid {
    let uid = init_http_session(&target_url).await;
    println!("HTTP Server copies. Established session {:#x?}", uid);

    let (s_read, mut s_write) = socket.into_split();

    let upload_join = {
        let target_url = target_url.clone();
        let s_read = artex(s_read);
        async move {
            #[allow(clippy::never_loop)]
            loop {
                let stream = WrapperBuilder {
                    guard: s_read.clone().try_lock_owned().unwrap(),
                    fr_builder: |a| FramedRead::new(a, BytesCodec::new()),
                }
                .build();
                upload_data(&target_url, uid, stream).await;
                //200
                break;
            }
        }
    };

    let download_join = {
        let target_url = target_url.clone();
        async move {
            #[allow(clippy::never_loop)]
            loop {
                let s = download_data(&target_url, uid).await;
                let r = s.map_err(|x| Err(x).unwrap()).into_async_read();
                let mut r = FuturesAsyncReadCompatExt::compat(r);
                tokio::io::copy(&mut r, &mut s_write).await.unwrap();
                break;
            }
        }
    };
    let upload_join = tokio::spawn(upload_join);
    let download_join = tokio::spawn(download_join);
    /*tokio::select!(
        x = upload_join => {
            Result::<(), _>::unwrap(x);
        }
        x = download_join => {
            Result::<(), _>::unwrap(x);
        }
    );*/
    let (up, down) = tokio::join!(upload_join, download_join);
    up.unwrap();
    down.unwrap();
    close_session(&target_url, uid).await;
    return uid;
}

pub async fn main(bind_addr: &[SocketAddr], target_url: Url) {
    //console_subscriber::init();
    let listener_result = TcpListener::bind(bind_addr).await;
    if let Err(bind_err) = listener_result {
        match bind_err.kind() {
            ErrorKind::AddrInUse => eprintln!(
                "Port {:?} is already in use.",
                bind_addr.iter().map(SocketAddr::port).collect::<Vec<_>>()
            ),
            ErrorKind::AddrNotAvailable => {
                eprintln!(
                    "Could not bind to IP {:?}. Not found.",
                    bind_addr.iter().map(SocketAddr::ip).collect::<Vec<_>>()
                );
            }
            ErrorKind::PermissionDenied => eprintln!(
                "Permission denied. Port {:?} to low for non-root user?",
                bind_addr.iter().map(SocketAddr::port).collect::<Vec<_>>()
            ),
            e => eprintln!(
                "Could not listen to your desired ip address or port: {:?}",
                e
            ),
        }
        return;
    };
    let listener = listener_result.unwrap();
    println!("Listening on {}", listener.local_addr().unwrap());
    let target_url = Arc::new(target_url);
    loop {
        let (socket, _) = listener.accept().await.unwrap();
        let target_url = target_url.clone();
        let _join_handle = tokio::spawn(async move {
            process_socket(target_url, socket).await;
        });
    }
}

#[track_caller]
fn assert_ok(resp: Response) -> impl std::future::Future<Output = Response> {
    #[cfg(feature = "rustc_stable")]
    return async move { resp };
    #[cfg(not(feature = "rustc_stable"))]
    {
        let blame_caller = std::intrinsics::caller_location();
        async move {
            if resp.status() == reqwest::StatusCode::OK {
                return resp;
            }
            let s = format!("{resp:#?}");
            let bytes = resp.bytes().await;
            let resp_body = bytes.as_ref().map(|x| std::str::from_utf8(x));
            eprintln!("{s}");
            eprintln!("{resp_body:#?}");
            eprintln!("caller: {blame_caller}");
            panic!("status was not ok");
        }
    }
}
