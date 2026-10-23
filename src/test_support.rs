//! Local HTTP fixtures. No external DNS, API, or environment proxy is used.

use std::io::{self, BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crate::provider::geoiplookup::Client;

const CLIENT_TIMEOUT: Duration = Duration::from_secs(1);
// Windows may take over a second to report a refused connection. Keep the short
// timeout above for stalled-response tests, but give connection refusal time.
const CONNECTION_REFUSED_TIMEOUT: Duration = Duration::from_secs(5);
const IO_TIMEOUT: Duration = Duration::from_secs(3);

pub enum Response {
    Http {
        status: u16,
        body: &'static str,
    },
    HttpWithType {
        status: u16,
        body: &'static str,
        content_type: &'static str,
    },
    Redirect {
        location: String,
    },
    Disconnect,
    StallBeforeHeaders,
    StallDuringBody,
}

pub struct TestServer {
    address: SocketAddr,
    stop: Arc<AtomicBool>,
    release: Sender<()>,
    requests: Receiver<String>,
    headers: Receiver<Vec<String>>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl TestServer {
    pub fn new(responses: Vec<Response>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (release, released) = mpsc::channel();
        let (requests_tx, requests) = mpsc::channel();
        let (headers_tx, headers) = mpsc::channel();
        let worker = thread::spawn(move || {
            for response in responses {
                if worker_stop.load(Ordering::SeqCst) {
                    break;
                }
                let (mut stream, _) = listener.accept()?;
                if worker_stop.load(Ordering::SeqCst) {
                    break;
                }
                stream.set_read_timeout(Some(IO_TIMEOUT))?;
                stream.set_write_timeout(Some(IO_TIMEOUT))?;
                let mut reader = BufReader::new(&stream);
                let mut request = String::new();
                if reader.read_line(&mut request)? == 0 {
                    return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                }
                let _ = requests_tx.send(request.trim_end().to_string());
                let mut request_headers = Vec::new();
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line)? == 0 {
                        return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
                    }
                    if line == "\r\n" {
                        break;
                    }
                    request_headers.push(line.trim_end().to_string());
                }
                let _ = headers_tx.send(request_headers);
                drop(reader);

                match response {
                    Response::Http { status, body } => {
                        write!(
                            stream,
                            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )?;
                    }
                    Response::HttpWithType {
                        status,
                        body,
                        content_type,
                    } => {
                        write!(
                            stream,
                            "HTTP/1.1 {status} Test\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )?;
                    }
                    Response::Redirect { location } => {
                        write!(
                            stream,
                            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        )?;
                    }
                    Response::Disconnect => {}
                    Response::StallBeforeHeaders => {
                        let _ = released.recv();
                    }
                    Response::StallDuringBody => {
                        stream.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n",
                        )?;
                        let _ = released.recv();
                    }
                }
            }
            Ok(())
        });

        Self {
            address,
            stop,
            release,
            requests,
            headers,
            worker: Some(worker),
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}", self.address)
    }

    pub fn client(&self) -> Client {
        Client::for_test(self.url(), CLIENT_TIMEOUT)
    }

    pub fn requests(&self) -> Vec<String> {
        self.requests.try_iter().collect()
    }

    pub fn headers(&self) -> Vec<Vec<String>> {
        self.headers.try_iter().collect()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.release.send(());
        // Unblock accept if a test failed before making all expected requests.
        let _ = TcpStream::connect_timeout(&self.address, CLIENT_TIMEOUT);
        if let Some(worker) = self.worker.take() {
            let result = worker.join();
            if !thread::panicking() {
                result
                    .expect("test server panicked")
                    .expect("test server I/O failed");
            }
        }
    }
}

pub fn unavailable_client() -> Client {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = Client::for_test(
        format!("http://{}", listener.local_addr().unwrap()),
        CONNECTION_REFUSED_TIMEOUT,
    );
    drop(listener);
    client
}
