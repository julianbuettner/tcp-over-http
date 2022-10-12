use crate::{
    entry,
    exit::{self, ExitSession, ExitSessionManager},
    init_panic_hook, ResolveAddr,
};
use actix_web::web;
use halfbrown::HashMap;
use itertools::Itertools;
use rand::RngCore;
use std::{convert::TryInto, net::SocketAddr, sync::atomic::Ordering, time::Duration};
use std::{ops::Deref, sync::Arc};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    join,
    net::TcpStream,
    sync::{MutexGuard, RwLockReadGuard},
    time::sleep,
};
use uuid::Uuid;

const TEST_SIZE: usize = (1024 * 1024 * 10) + 42;

struct Persist {
    entry_conn: TcpStream,
    exit_conn: TcpStream,
}

async fn roundtrip() -> Persist {
    let localhost = localhost().await;

    let target_listen = tokio::net::TcpListener::bind(localhost).await.unwrap();
    let (exit_addr, f_exit) = exit::main(localhost, vec![target_listen.local_addr().unwrap()]);

    let exit_addr = exit_addr.first().unwrap();

    let (entry_addr, f_entry) = entry::main(
        localhost,
        format!("http://{exit_addr}/").as_str().try_into().unwrap(),
    )
    .await;

    let f_test = async {
        //get rand
        let irand = {
            let mut rand_buf = vec![0; TEST_SIZE];
            rand::rngs::mock::StepRng::new(0, 1).fill_bytes(&mut rand_buf);
            rand_buf
        };
        //connect to entry
        let mut entry_conn = TcpStream::connect(entry_addr).await.unwrap();
        //send rand
        let join_send = async {
            entry_conn.write_all(&irand).await.unwrap();
        };
        //recv rand
        let mut exit_conn = target_listen.accept().await.unwrap().0;
        let mut post = vec![0; TEST_SIZE];
        let join_recv = async {
            exit_conn.read_exact(&mut post).await.unwrap();
        };
        //
        join!(join_send, join_recv);
        //assert rand
        assert!(post == irand);

        //make response
        let response = {
            let mut response_buf = irand;
            rand::rngs::mock::StepRng::new(TEST_SIZE.try_into().unwrap(), 1)
                .fill_bytes(&mut response_buf);
            response_buf
        };
        //send respond
        let join_send = async {
            exit_conn.write_all(&response).await.unwrap();
        };
        //recv response
        let join_recv = async {
            entry_conn.read_exact(&mut post).await.unwrap();
        };
        //
        join!(join_send, join_recv);
        //assert response
        assert!(post == response);

        Persist {
            entry_conn,
            exit_conn,
        }
    };

    tokio::select! {
        _ = f_exit => {
            panic!()
        }
        _ = f_entry => {
            panic!()
        }
        x = f_test => {
            return x;
        }
    };
}

#[test]
fn test() {
    init_panic_hook();

    RT.block_on(async {
        #[ouroboros::self_referencing]
        struct Guard {
            o: MutexGuard<'static, Option<web::Data<ExitSessionManager>>>,
            #[borrows(o)]
            #[covariant]
            r: RwLockReadGuard<'this, HashMap<Uuid, ExitSession>>,
        }
        impl Deref for Guard {
            type Target = HashMap<Uuid, ExitSession>;
            fn deref(&self) -> &Self::Target {
                #[allow(clippy::explicit_deref_methods)]
                self.borrow_r().deref()
            }
        }
        let lock = || async {
            let guard: Guard = GuardAsyncBuilder {
                o: crate::exit::test::ARC.lock().await,
                r_builder: |x| Box::pin(x.as_ref().unwrap().sessions.read()),
            }
            .build()
            .await;
            guard
        };

        #[allow(clippy::async_yields_async)]
        let assert = || async {
            let guard = lock().await;
            let one = guard.iter().at_most_one().unwrap().unwrap();
            let sess = one.1;
            let tcp_in = &sess.down.tcp_in;
            assert_eq!(Arc::strong_count(tcp_in), 2);
            let tcp_out = &sess.up.tcp_out;
            assert_eq!(Arc::strong_count(tcp_out), 2);
            tcp_in.try_lock().unwrap_err();
            tcp_out.try_lock().unwrap_err();
            assert!(!sess.up.stop_copy.is_cancelled());
            let tcp_in = tcp_in.clone();
            let tcp_out = tcp_out.clone();

            async move {
                assert_eq!(Arc::strong_count(&tcp_in), 1);
                assert_eq!(Arc::strong_count(&tcp_out), 1);
                drop(tcp_in.try_lock().unwrap());
                drop(tcp_out.try_lock().unwrap());
                assert_eq!(lock().await.len(), 0);

                assert_eq!(crate::entry::AC.load(Ordering::SeqCst), 0);
            }
        };

        let roundtrip = |rst| async move {
            let conn = roundtrip().await;
            if rst {
                conn.entry_conn.set_linger(Some(Duration::ZERO)).unwrap();
                conn.exit_conn.set_linger(Some(Duration::ZERO)).unwrap();
            }
            (conn.entry_conn, conn.exit_conn, assert().await)
        };

        let (entry_conn, exit_conn, assert) = roundtrip(false).await;
        drop((entry_conn, exit_conn));
        sleep(Duration::from_millis(100)).await;
        assert.await;

        let roundtrip = || roundtrip(true);

        dbg!();

        let (entry_conn, exit_conn, assert) = roundtrip().await;
        drop(entry_conn);
        sleep(Duration::from_millis(100)).await;
        assert.await;
        drop(exit_conn);

        dbg!();

        let (entry_conn, exit_conn, assert) = roundtrip().await;
        drop(exit_conn);
        sleep(Duration::from_millis(100)).await;
        assert.await;
        drop(entry_conn);

        dbg!();

        let (entry_conn, exit_conn, assert) = roundtrip().await;

        drop((entry_conn, exit_conn));
        sleep(Duration::from_millis(100)).await;
        assert.await;

        dbg!();
    });
}

#[test]
fn resolve() {
    RT.block_on(async {
        let addr = ResolveAddr("localhost:0".to_owned()).resolve().await;
        let addr = addr.into_iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(addr, ["[::1]:0", "127.0.0.1:0"]);
    });
}

lazy_static::lazy_static! {
    static ref RT: tokio::runtime::Runtime = tokio::runtime::Runtime::new().unwrap();
}

async fn localhost() -> &'static [SocketAddr] {
    static ONCE: tokio::sync::OnceCell<Vec<SocketAddr>> = tokio::sync::OnceCell::const_new();

    ONCE.get_or_init(|| async {
        tokio::net::lookup_host("localhost:0")
            .await
            .unwrap()
            .collect_vec()
    })
    .await
}
