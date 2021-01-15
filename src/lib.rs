#![cfg(test)]

use std::error::Error as StdError;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::net::TcpListener as StdTcpListener;

use bytes::Buf;
use bytes::Bytes;
use futures::stream;
use http::{Request, Response};
use hyper::Body;
use hyper::client::{Client, HttpConnector};
use hyper::server::conn::Http;
use hyper::service::service_fn;

use tokio::net::TcpListener;
use tokio::spawn;

pub type Flaw = Box<dyn StdError + Send + Sync + 'static>;

// Return a tuple of (serv: impl Future, url: String) that will service C
// requests via function, and the url to access it via a local tcp port.
macro_rules! service {
    ($c:literal, $s:ident) => {{
        let (listener, addr) = local_bind().unwrap();
        let fut = async move {
            for i in 0..$c {
                eprintln!("service! accepting...");
                let socket = listener.accept()
                    .await
                    .expect("accept").0;

                #[cfg(feature = "no-delay")]
                {
                    eprintln!("service! setting nodelay");
                    socket.set_nodelay(true).expect("nodelay");
                }

                eprintln!("service! accepted, serve...");
                let res = Http::new()
                    .serve_connection(socket, service_fn($s))
                    .await;
                if let Err(e) = res {
                    eprintln!("On service! [{}]: {}", i, e);
                    break;
                }
            }
            eprintln!("service! completing");
        };
        (format!("http://{}", &addr), fut)
    }}
}

#[test]
fn streaming_echo() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(2)
        .enable_io()
        .enable_time()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let (url, srv) = service!(1, echo);
        let jh = spawn(srv);

        let s: Vec<Result<Bytes, hyper::Error>> = vec![
            Ok(Bytes::from("chunk1")),
            Ok(Bytes::from("chunk2"))];
        let s = stream::iter(s);
        let body = hyper::Body::wrap_stream(s);
        // let body = "chunk1chunk2".into();
        let res = spawn(post_body_req(&url, body))
            .await
            .unwrap();

        match res {
            Ok(resp) => {
                let buf = hyper::body::aggregate(resp.into_body())
                    .await
                    .unwrap();
                assert_eq!(buf.remaining(), 6 + 6);
            }
            Err(e) => {
                panic!("failed with: {}", e);
            }
        }

        let _ = jh .await;

    });
}

fn local_bind() -> Result<(TcpListener, SocketAddr), io::Error> {
    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let std_listener = StdTcpListener::bind(addr).unwrap();
    let listener = TcpListener::from_std(std_listener)?;
    let local_addr = listener.local_addr()?;
    Ok((listener, local_addr))
}

async fn echo(req: Request<Body>) -> Result<Response<Body>, hyper::Error> {
    Ok(Response::new(req.into_body()))
}

fn post_body_req(url: &str, body: Body)
    -> impl Future<Output=Result<Response<Body>, hyper::Error>> + Send
{
    let req: Request<Body> = http::Request::builder()
        .method(http::Method::POST)
        .uri(url)
        .body(body)
        .unwrap();
    let client = Client::builder().build(HttpConnector::new());
    client.request(req)
}
