use crate::{entry, exit, set_panic_hook, ResolveAddr};
use std::convert::TryInto;
use std::net::SocketAddr;
use std::str::FromStr;
use tokio::sync::Mutex;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    join,
};

#[test]
fn test() {
    static RAND_BUF: Mutex<Option<Vec<u8>>> = Mutex::const_new(None);
    const TEST_SIZE: usize = (1024 * 1024) + 42;

    set_panic_hook();

    let test = async {
        let tcp_listen = tokio::net::TcpListener::bind("127.0.0.1:1234")
            .await
            .unwrap();
        let addr = &[SocketAddr::from_str("127.0.0.1:8080").unwrap()];
        let f_exit = exit::main(addr, vec![SocketAddr::from_str("127.0.0.1:1234").unwrap()]);
        let addr = &[SocketAddr::from_str("127.0.0.1:1415").unwrap()];
        let f_entry = entry::main(addr, "http://127.0.0.1:8080/".try_into().unwrap());
        let f_test = async {
            let entry = loop {
                if let Ok(x) = tokio::net::TcpStream::connect("127.0.0.1:1415").await {
                    break x;
                }
            };
            let (mut entry_out, entry_in) = entry.into_split();
            let mut dev_rand = tokio::fs::File::open("/dev/urandom").await.unwrap();

            let mut lock = RAND_BUF.try_lock().unwrap();
            *lock = Some(vec![0; TEST_SIZE]);
            //read rand
            dev_rand.read_exact(lock.as_mut().unwrap()).await.unwrap();
            //send rand
            let join_send = tokio::spawn({
                //let rand_buf = rand_buf.clone();
                async {
                    //let rand_buf = rand_buf;
                    let lock = lock;
                    let mut entry_in = entry_in;
                    entry_in.write_all(lock.as_ref().unwrap()).await.unwrap();
                }
            });
            //recv rand
            let mut tcp_conn = tcp_listen.accept().await.unwrap().0;
            let mut post = vec![0; TEST_SIZE];
            tcp_conn.read_exact(&mut post).await.unwrap();
            //should be done by here
            join_send.await.unwrap();
            //assert rand
            assert_eq!(post.len(), TEST_SIZE);
            assert!(&post == RAND_BUF.try_lock().unwrap().as_ref().unwrap());

            let fs = async {
                //respond rand
                tcp_conn.write_all(&post).await.unwrap();
            };
            let fr = async {
                //receive rand
                let mut out = vec![0; TEST_SIZE];
                entry_out.read_exact(&mut out).await.unwrap();
            };

            join!(fs, fr);
        };
        tokio::select! {
            _ = f_exit => {
                panic!()
            }
            _ = f_entry => {
                panic!()
            }
            _ = f_test => {}
        };
    };
    tokio::runtime::Runtime::new().unwrap().block_on(test);
}

#[test]
fn resolve() {
    let resolve = async {
        let addr = ResolveAddr("localhost:0".to_owned()).resolve().await;
        let addr = addr.into_iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(addr, ["[::1]:0", "127.0.0.1:0"]);
    };
    tokio::runtime::Runtime::new().unwrap().block_on(resolve);
}
