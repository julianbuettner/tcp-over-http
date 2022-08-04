use super::code::{decode, encode};
use super::error::EntryNodeError;
use hyper::{Body, Client, Request, StatusCode};
use std::io::ErrorKind;
use std::str::FromStr;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::task;

async fn body_into_string(b: Body) -> Result<String, EntryNodeError> {
    Ok(String::from_utf8(hyper::body::to_bytes(b).await?.to_vec()).unwrap())
}

async fn close_session<
    T: hyper::client::connect::Connect + Clone + std::marker::Send + Sync + 'static,
>(
    target: &String,
    client: &Client<T>,
    uid: u128,
) -> Result<(), EntryNodeError> {
    let req = Request::post(&format!("{}close/{}", target, uid))
        .body(Body::empty())
        .unwrap();
    let res = client.request(req).await?;

    if res.status() != StatusCode::OK {
        return Err(EntryNodeError::f(format!(
            "HTTP Exit node error status code: {}\n{:?}",
            res.status(),
            body_into_string(res.into_body()).await?,
        )));
    }
    Ok(())
}
async fn init_http_session<
    T: hyper::client::connect::Connect + Clone + std::marker::Send + Sync + 'static,
>(
    target: &String,
    client: &Client<T>,
) -> Result<u128, EntryNodeError> {
    let req = Request::post(&format!("{}open", target))
        .body(Body::empty())
        .unwrap();
    let res = client.request(req).await?;

    if res.status() != StatusCode::OK {
        return Err(EntryNodeError::f(format!(
            "HTTP Exit node error status code: {}\n{:?}",
            res.status(),
            body_into_string(res.into_body()).await?,
        )));
    }
    let uid = u128::from_str(&body_into_string(res.into_body()).await?)?;
    Ok(uid)
}

async fn upload_data<
    T: hyper::client::connect::Connect + Clone + std::marker::Send + Sync + 'static,
>(
    target: &String,
    uid: u128,
    client: &Client<T>,
    data: Vec<u8>,
) -> Result<(), EntryNodeError> {
    let req = Request::post(&format!("{}u/{}", target, uid))
        .body(Body::from(encode(data)))
        .unwrap();
    let res = client.request(req).await?;
    if res.status() != StatusCode::OK {
        return Err(EntryNodeError::f(format!(
            "Failed to upload data to HTTP Server.\n{:?}",
            body_into_string(res.into_body()).await?,
        )));
    }
    Ok(())
}

async fn download_data<
    T: hyper::client::connect::Connect + Clone + std::marker::Send + Sync + 'static,
>(
    target: &String,
    uid: u128,
    client: &Client<T>,
) -> Result<Vec<u8>, EntryNodeError> {
    let req = Request::post(&format!("{}s/{}", target, uid))
        .body(Body::empty())
        .unwrap();
    let res = client.request(req).await?;
    if res.status() != StatusCode::OK {
        return Err(EntryNodeError::f(format!(
            "Failed to download data from HTTP Server.\n{:?}",
            body_into_string(res.into_body()).await?,
        )));
    }
    Ok(decode(body_into_string(res.into_body()).await?)?)
}

async fn process_socket(
    target_url: String,
    socket: tokio::net::TcpStream,
) -> Result<u128, EntryNodeError> {
    let mut socket = socket;
    let mut buf: Vec<u8> = vec![0; 1024];
    let client = Client::new();
    let uid = init_http_session(&target_url, &client).await?;
    println!("HTTP Server copies. Established session {}", uid);
    let mut download_join_handle = task::spawn(async { Ok(Vec::<u8>::new()) });
    loop {
        if download_join_handle.is_finished() {
            let data = match download_join_handle.await {
                Err(e) => Err(EntryNodeError::f(format!("{:?}", e))),
                Ok(v) => Ok(v),
            }??;
            print!(".");
            // println!(
            //     "{:0>3} From HTTP for TCP connection: {}",
            //     uid % 1000,
            //     data.len()
            // );
            socket.write_all(&data).await.expect("Please never happen");
            let client = client.clone();
            let target_url = target_url.clone();
            download_join_handle = task::spawn(async move {
                let res = download_data(&target_url, uid, &client).await;
                res
            });
        }

        let read_trial = socket.try_read(&mut buf);
        let bytes_read_count = match read_trial {
            Ok(0) => {
                close_session(&target_url, &client, uid).await?;
                Err(EntryNodeError::f(format!("Read socket closed")))
            }
            Ok(n) => Ok(n),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                tokio::task::yield_now().await;
                Ok(0)
            }
            Err(e) => Err(EntryNodeError::f(format!("Sleepdeprevation {:?}", e))),
        }?;
        if bytes_read_count > 0 {
            print!("-");
            // println!(
            //     "{:0>3} From TCP to HTTP. Uploading now: {}B",
            //     uid % 1000,
            //     bytes_read_count
            // );
            upload_data(
                &target_url,
                uid,
                &client,
                (&buf[0..bytes_read_count]).to_vec(),
            )
            .await?;
        } else {
            tokio::task::yield_now().await;
        }
    }
    // close_session(&target_url, &client, uid).await?;
    // println!("{:0>3} Connection closed", uid % 1000);
    // Ok(uid)
}

pub async fn entry_main(bind_ip: std::net::IpAddr, listen_tcp_port: u16, target_url: String) {
    let bind_uri = format!("{}:{}", bind_ip, listen_tcp_port);
    let listener_result = TcpListener::bind(&bind_uri).await;
    if let Err(bind_err) = listener_result {
        match bind_err.kind() {
            ErrorKind::AddrInUse => println!("Port {} is already in use.", listen_tcp_port),
            ErrorKind::AddrNotAvailable => println!("Could not bind to IP {}. Not found.", bind_ip),
            ErrorKind::PermissionDenied => println!(
                "Permission denied. Port {} to low for non-root user?",
                listen_tcp_port
            ),
            e => println!(
                "Could not listen to your desired ip address or port: {:?}",
                e
            ),
        }
        return;
    };
    let listener = listener_result.unwrap();
    println!("Listening on {}", bind_uri);
    loop {
        let (socket, _) = listener.accept().await.unwrap();
        let target_url = target_url.clone();
        let _join_handle = tokio::task::spawn(async {
            let _result = process_socket(target_url, socket).await;
        });
    }
}
