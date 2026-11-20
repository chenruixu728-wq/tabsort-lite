use crate::pipe;
use crate::protocol::{receive, send, Hello, Response, Snapshot, PROTOCOL};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::fs::File;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

#[derive(Clone)]
pub struct Browser {
    pub connection: u64,
    pub snapshot: Snapshot,
    pub group_split: bool,
    requests: Sender<Request>,
}

pub enum Event {
    Connected(Browser),
    Disconnected(u64),
    ListenerError(String),
}

struct Request {
    kind: &'static str,
    data: Value,
    reply: Sender<Result<Value>>,
}

impl Browser {
    pub fn request(&self, kind: &'static str, data: Value) -> Result<Value> {
        let (sender, receiver) = mpsc::channel();
        self.requests
            .send(Request {
                kind,
                data,
                reply: sender,
            })
            .map_err(|_| anyhow::anyhow!("Edge 已断开连接"))?;
        receiver
            .recv_timeout(Duration::from_secs(120))
            .context("Edge 连接中断或长时间未响应")?
    }
}

pub fn listen() -> Receiver<Event> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        for connection in 1.. {
            match pipe::accept() {
                Ok(file) => {
                    let sender = sender.clone();
                    std::thread::spawn(move || {
                        let _ = serve(connection, file, &sender);
                        let _ = sender.send(Event::Disconnected(connection));
                    });
                }
                Err(error) => {
                    let _ =
                        sender.send(Event::ListenerError(format!("无法启动 Edge 连接：{error}")));
                    break;
                }
            }
        }
    });
    receiver
}

fn serve(connection: u64, mut file: File, events: &Sender<Event>) -> Result<()> {
    let hello: Hello = receive(&mut file)?;
    if hello.protocol != PROTOCOL || !valid_id(&hello.profile_id) || !valid_id(&hello.session_id) {
        bail!("Edge 连接身份无效");
    }
    let snapshot: Snapshot = serde_json::from_value(exchange(&mut file, 1, "scan", Value::Null)??)?;
    if snapshot.profile_id != hello.profile_id || snapshot.session_id != hello.session_id {
        bail!("Edge 连接身份已变化");
    }
    let (sender, receiver) = mpsc::channel();
    events.send(Event::Connected(Browser {
        connection,
        snapshot,
        group_split: hello.group_split,
        requests: sender,
    }))?;
    for id in 2.. {
        match receiver.recv_timeout(Duration::from_secs(3)) {
            Ok(request) => match exchange(&mut file, id, request.kind, request.data) {
                Ok(result) => {
                    let _ = request.reply.send(result);
                }
                Err(error) => {
                    let _ = request.reply.send(Err(error));
                    break;
                }
            },
            Err(RecvTimeoutError::Timeout) => {
                exchange(&mut file, id, "ping", Value::Null)??;
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn exchange(file: &mut File, id: u64, kind: &str, data: Value) -> Result<Result<Value>> {
    send(file, &json!({"id": id, "type": kind, "data": data})).context("无法向 Edge 发送指令")?;
    let response: Response = receive(file).context("Edge 未完成指令，请检查浏览器连接")?;
    if response.id != id {
        bail!("Edge 返回了不匹配的指令结果");
    }
    if response.ok {
        Ok(Ok(response.data.unwrap_or(Value::Null)))
    } else {
        Ok(Err(anyhow::anyhow!(response
            .error
            .unwrap_or_else(|| "Edge 指令失败".into()))))
    }
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
}
