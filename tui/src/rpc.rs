use serde_json::Value;
use std::{
    io::{self, BufRead, BufReader, Write},
    net::TcpStream,
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

pub enum RpcEvent {
    Response { id: String, result: Value },
    Error { id: Option<String>, error: Value },
    Notification { method: String, params: Value },
    Closed { message: String },
}

pub struct RpcClient {
    sender: Sender<String>,
    receiver: Receiver<RpcEvent>,
    _child: Option<Child>,
}

impl RpcClient {
    pub fn connect_local(imsg_bin: &str, db_path: Option<&str>) -> io::Result<Self> {
        let mut cmd = Command::new(imsg_bin);
        cmd.arg("rpc");
        if let Some(db) = db_path {
            cmd.arg("--db").arg(db);
        }
        cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "failed to open stdin")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "failed to open stdout")
        })?;
        let (sender, receiver) = connect_with_io(stdin, stdout);
        Ok(Self {
            sender,
            receiver,
            _child: Some(child),
        })
    }

    pub fn connect_tcp(host: &str, port: u16) -> io::Result<Self> {
        let stream = TcpStream::connect((host, port))?;
        let write_stream = stream.try_clone()?;
        let (sender, receiver) = connect_with_io(write_stream, stream);
        Ok(Self {
            sender,
            receiver,
            _child: None,
        })
    }

    pub fn send_request(&mut self, method: &str, params: Option<Value>) -> String {
        let id = next_id();
        let mut payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
        });
        if let Some(params) = params {
            if let Some(obj) = payload.as_object_mut() {
                obj.insert("params".to_string(), params);
            }
        }
        let line = payload.to_string();
        let _ = self.sender.send(line);
        id
    }

    pub fn events(&self) -> &Receiver<RpcEvent> {
        &self.receiver
    }
}

fn connect_with_io<W: Write + Send + 'static, R: io::Read + Send + 'static>(
    writer: W,
    reader: R,
) -> (Sender<String>, Receiver<RpcEvent>) {
    let (tx, rx) = mpsc::channel::<String>();
    let (event_tx, event_rx) = mpsc::channel::<RpcEvent>();

    thread::spawn(move || writer_thread(writer, rx));
    thread::spawn(move || reader_thread(reader, event_tx));

    (tx, event_rx)
}

fn writer_thread<W: Write>(mut writer: W, rx: Receiver<String>) {
    for line in rx {
        if writeln!(writer, "{line}").is_err() {
            break;
        }
        let _ = writer.flush();
    }
}

fn reader_thread<R: io::Read>(reader: R, event_tx: Sender<RpcEvent>) {
    let buffered = BufReader::new(reader);
    for line in buffered.lines().flatten() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&line) {
            Ok(value) => {
                if let Some(method) = value.get("method").and_then(|v| v.as_str()) {
                    if let Some(params) = value.get("params") {
                        let _ = event_tx.send(RpcEvent::Notification {
                            method: method.to_string(),
                            params: params.clone(),
                        });
                        continue;
                    }
                }
                if let Some(id) = value.get("id").and_then(|v| v.as_str()) {
                    if let Some(result) = value.get("result") {
                        let _ = event_tx.send(RpcEvent::Response {
                            id: id.to_string(),
                            result: result.clone(),
                        });
                        continue;
                    }
                    if let Some(error) = value.get("error") {
                        let _ = event_tx.send(RpcEvent::Error {
                            id: Some(id.to_string()),
                            error: error.clone(),
                        });
                        continue;
                    }
                }
            }
            Err(err) => {
                let _ = event_tx.send(RpcEvent::Closed {
                    message: format!("json parse error: {err}"),
                });
                break;
            }
        }
    }
    let _ = event_tx.send(RpcEvent::Closed {
        message: "rpc stream closed".to_string(),
    });
}

fn next_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed).to_string()
}
