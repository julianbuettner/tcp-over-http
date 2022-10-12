use crate::error::Trace;
use crate::ouroboros_impl_wrapper::WrapperBuilder;
use crate::{artex, join_url};

use bytes::Bytes;
use futures::Future;
use reqwest::{Body, Client, Response, Url};
use std::convert::{identity, Infallible, TryInto};
use std::io::ErrorKind;
use std::net::SocketAddr;
use std::sync::Arc;
use stream_cancel::Valve;
use tokio::net::TcpListener;
use tokio_stream::StreamExt;
use tokio_util::codec::{BytesCodec, FramedRead};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

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
async fn init_http_session(target: &Url) -> Trace<Uuid> {
    let resp = CLIENT.get(join_url(target, ["open"])).send().await.unwrap();
    let resp = assert_ok(resp).await;
    return Ok(Uuid::from_bytes(
        match identity::<&[u8]>(&resp.bytes().await.unwrap()).try_into() {
            Ok(x) => x,
            Err(x) => {
                use crate::error::ContextExt;
                //signal from exit
                return Err(x.with_context("couldnt connect to target"));
            }
        },
    ));
}

async fn upload_req<S>(target: &Url, uid: Uuid, data: S)
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
    let resp = assert_ok(resp).await;
    dbg!(resp.text().await.unwrap());
}

async fn download_req(
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

async fn process_socket(target_url: Arc<Url>, socket: tokio::net::TcpStream) -> Trace<Uuid> {
    let uid = init_http_session(&target_url).await?;
    println!("HTTP Server copies. Established session {uid:#x?}");

    let (s_read, mut s_write) = socket.into_split();

    let stop_download = CancellationToken::new();
    let (stop_upload, valve) = Valve::new();

    let upload_join = {
        let stop_download = stop_download.clone();
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

                let stream = stream.map_while::<Result<_, anyhow::Error>, _>(|x| match x {
                    Ok(x) => Some(Ok(x)),
                    Err(x) => {
                        dbg!(x);
                        None
                    }
                });
                let stream = valve.wrap(stream);
                upload_req(&target_url, uid, stream).await;
                //200
                break;
            }
            stop_download.cancel();
        }
    };

    let download_join = {
        let target_url = target_url.clone();
        async move {
            #[allow(clippy::never_loop)]
            loop {
                use futures::TryStreamExt;
                use tokio_util::compat::FuturesAsyncReadCompatExt;

                let s = download_req(&target_url, uid).await;
                let mut r = s
                    .map_while(|x| match x {
                        Ok(x) => Some(Ok(x)),
                        Err(x) => {
                            dbg!(x);
                            None
                        }
                    })
                    .into_async_read()
                    .compat();

                tokio::select! {
                    x = tokio::io::copy(&mut r, &mut s_write) => x.expect("unreachable"),
                    _ = stop_download.cancelled() => break,
                };

                break;
            }
            stop_upload.cancel();
        }
    };
    let upload_join = tokio::spawn(upload_join);
    let download_join = tokio::spawn(download_join);
    upload_join.await.unwrap();
    download_join.await.unwrap();
    close_session(&target_url, uid).await;
    return Ok(uid);
}

pub async fn main(
    bind_addr: &[SocketAddr],
    target_url: Url,
) -> (SocketAddr, impl Future<Output = Infallible>) {
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
        panic!();
    };
    let listener = listener_result.unwrap();
    let bound = listener.local_addr().unwrap();
    println!("Listening on {bound}");
    let target_url = Arc::new(target_url);
    return (bound, async move {
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            let target_url = target_url.clone();
            let _join_handle = tokio::spawn(async move {
                #[cfg(test)]
                AC.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                drop(dbg!(process_socket(target_url, socket).await));
                #[cfg(test)]
                AC.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            });
        }
    });
}

#[cfg(test)]
pub(crate) static AC: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

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
