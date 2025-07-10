use std::convert::Infallible;
use std::net::SocketAddr;

use http_body_util::Full;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{
    body::{Body, Bytes},
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;

fn not_found() -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from("Not Found")));
    *response.status_mut() = StatusCode::NOT_FOUND;
    response.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("text/plain"),
    );
    response
}

fn ping(_req: Request<impl Body>) -> Response<Full<Bytes>> {
    Response::new(Full::new(Bytes::from("pong")))
}

async fn handler(req: Request<hyper::body::Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&Method::GET, "/ping") => Ok(ping(req)),
        _ => Ok(not_found()),
    }
}

pub async fn run(port: u16) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let addr: SocketAddr = ([127, 0, 0, 1], port).into();
    let listener = TcpListener::bind(addr).await?;

    println!("API is listening on http://{}", addr);
    loop {
        let (tcp, _) = listener.accept().await?;
        let io = TokioIo::new(tcp);
        tokio::task::spawn(async move {
            if let Err(err) = http1::Builder::new()
                .serve_connection(io, service_fn(handler))
                .await
            {
                println!("Error serving connection: {:?}", err);
            }
        });
    }
}
