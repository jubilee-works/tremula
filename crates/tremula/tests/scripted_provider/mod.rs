//! A model provider that answers from a script, over a real socket.
//!
//! Shared by the tests of the reference adapter, which each use a part of it:
//! one asks what the adapter sends, the other what it does with what comes back.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]
// Setting a variable of the environment is unsafe in this edition, and reading
// the key from the environment at call time is the contract under test. Every
// test that touches it holds `ENVIRONMENT` for the duration.
#![allow(unsafe_code)]

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex, PoisonError},
    thread,
    time::Duration,
};

use serde_json::{Value, json};
use tremula::generate::{CoveringTest, GenerationRequest, openai::OpenAiGenerator};

/// The model every scripted call names.
pub const MODEL: &str = "gpt-5.2-2025-12-11";

/// A key shaped like the real thing, so a leak would be visible.
pub const FAKE_KEY: &str = "sk-test-3f9a2b7c1d4e6f8a0b2c4d6e8f0a2b4c";

/// Serialises the tests that read the key out of the environment.
pub static ENVIRONMENT: Mutex<()> = Mutex::new(());

/// One scripted answer.
pub struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
    silent: bool,
}

impl Reply {
    /// A successful answer carrying `body`.
    pub fn ok(body: String) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body,
            silent: false,
        }
    }

    /// A refusal with a status of its own.
    pub fn refusing(status: u16, body: &str) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.to_owned(),
            silent: false,
        }
    }

    /// The same, with a header.
    #[must_use]
    pub fn and_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// A connection that is accepted and then never answered.
    pub fn silence() -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: String::new(),
            silent: true,
        }
    }
}

/// A socket that hands out replies in order and keeps what it was sent.
pub struct Provider {
    endpoint: String,
    seen: Arc<Mutex<Vec<String>>>,
}

impl Provider {
    /// Start one, listening on a port the operating system chooses.
    pub fn scripted(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "http://{}/v1/chat/completions",
            listener.local_addr().unwrap()
        );
        let seen = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&seen);
        thread::spawn(move || {
            for reply in replies {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                recorded.lock().unwrap().push(read_request(&mut stream));
                if reply.silent {
                    thread::sleep(Duration::from_secs(3));
                    continue;
                }
                // Every answer closes its connection, so that a retry is a new
                // one and arrives at the next accept rather than being served
                // from a socket this loop has already moved past.
                let mut answer = format!(
                    "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                    reply.status,
                    reply.body.len()
                );
                for (name, value) in &reply.headers {
                    answer.push_str(name);
                    answer.push_str(": ");
                    answer.push_str(value);
                    answer.push_str("\r\n");
                }
                answer.push_str("\r\n");
                answer.push_str(&reply.body);
                let _ = stream.write_all(answer.as_bytes());
                let _ = stream.flush();
            }
        });
        Self { endpoint, seen }
    }

    /// A generator pointed at this socket.
    pub fn generator(&self) -> OpenAiGenerator {
        OpenAiGenerator::new(MODEL)
            .with_endpoint(self.endpoint.clone())
            .with_timeout(Duration::from_millis(600))
    }

    /// How many calls arrived.
    pub fn calls(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    /// The payload of one call, parsed.
    pub fn payload(&self, which: usize) -> Value {
        let request = self.request(which);
        let (_, body) = request
            .split_once("\r\n\r\n")
            .expect("a request with a body");
        serde_json::from_str(body).expect("a payload of JSON")
    }

    /// One call as it arrived, headers and body both.
    pub fn request(&self, which: usize) -> String {
        let seen = self.seen.lock().unwrap();
        seen.get(which).expect("that call never arrived").clone()
    }
}

/// Read one HTTP request, headers and body both.
fn read_request(stream: &mut TcpStream) -> String {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut head = String::new();
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        if let Some(value) = line.to_lowercase().strip_prefix("content-length:") {
            length = value.trim().parse().unwrap_or(0);
        }
        let blank = line == "\r\n";
        head.push_str(&line);
        if blank {
            break;
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).unwrap_or_default();
    head + &String::from_utf8_lossy(&body)
}

/// A well-formed answer with `count` mutations in it.
pub fn answer(count: usize) -> String {
    let mutants: Vec<Value> = (0..count)
        .map(|which| {
            json!({
                "file": "ranges.py",
                "original": format!("start < other_end and other_start < end # {which}"),
                "replacement": "start <= other_end and other_start < end",
                "description": "an off-by-one at the boundary a test might not pin",
            })
        })
        .collect();
    json!({ "mutants": mutants }).to_string()
}

/// The envelope a provider wraps an answer in.
pub fn envelope(content: &str, finish: &str) -> String {
    json!({
        "id": "chatcmpl-scripted",
        "model": MODEL,
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": content, "refusal": null},
            "finish_reason": finish,
        }],
        "usage": {"prompt_tokens": 682, "completion_tokens": 436, "total_tokens": 1118},
    })
    .to_string()
}

/// The request every test asks about.
pub fn request(mutant_count: usize) -> GenerationRequest {
    GenerationRequest {
        file: "ranges.py".to_owned(),
        source: "def overlaps(start, end, other_start, other_end):\n    return start < other_end and other_start < end\n".to_owned(),
        tests: vec![CoveringTest {
            file: "test_ranges.py".to_owned(),
            source: "def test_touching_ranges_do_not_overlap():\n    assert not overlaps(0, 30, 30, 60)\n".to_owned(),
        }],
        excluded: Vec::new(),
        mutant_count,
        feedback: None,
    }
}

/// Run `body` with the key in the environment, then put the environment back.
pub fn with_key<T>(key: Option<&str>, body: impl FnOnce() -> T) -> T {
    let guard = ENVIRONMENT.lock().unwrap_or_else(PoisonError::into_inner);
    let before = std::env::var(tremula::generate::openai::KEY_VARIABLE).ok();
    set_key(key);
    let outcome = body();
    set_key(before.as_deref());
    drop(guard);
    outcome
}

/// Put the key in the environment, or take it out.
fn set_key(key: Option<&str>) {
    let variable = tremula::generate::openai::KEY_VARIABLE;
    match key {
        Some(value) => unsafe { std::env::set_var(variable, value) },
        None => unsafe { std::env::remove_var(variable) },
    }
}
