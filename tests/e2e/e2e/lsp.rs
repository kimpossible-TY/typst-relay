use std::{
    collections::{HashMap, HashSet},
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use sync_ls::{lsp, RequestId};

use crate::artifact::{cli, GIT_ROOT};

#[test]
fn test_lsp() {
    std::env::set_var("RUST_BACKTRACE", "full");
    let root = GIT_ROOT.join("target/e2e/tinymist");

    {
        gen_smoke(SmokeArgs {
            root: root.join("neovim"),
            init: "initialization/neovim-0.9.4".to_owned(),
            log: "tests/fixtures/editions/neovim_unnamed_buffer.log".to_owned(),
        });

        let hash = replay_log(&root.join("neovim"));
        insta::assert_snapshot!(hash, @"siphash128_13:3d874d884ffadc490b30e6556ca535e6");
    }

    {
        gen_smoke(SmokeArgs {
            root: root.join("vscode"),
            init: "initialization/vscode-1.87.2".to_owned(),
            log: "tests/fixtures/editions/base.log".to_owned(),
        });

        let hash = replay_log(&root.join("vscode"));
        insta::assert_snapshot!(hash, @"siphash128_13:d105f02f35a3753d08ddd1ed2c6ac803");
    }

    {
        gen_smoke(SmokeArgs {
            root: root.join("vscode-syntax-only"),
            init: "initialization/vscode-syntax-only-1.87.2".to_owned(),
            log: "tests/fixtures/editions/base.log".to_owned(),
        });

        let hash = replay_log(&root.join("vscode-syntax-only"));
        insta::assert_snapshot!(hash, @"siphash128_13:d8e330d273d6c3784d142f545a5f7053");
    }
}

fn handle_io<T>(res: io::Result<T>) -> T {
    match res {
        Ok(status) => status,
        Err(err) => panic!("Error: {err}"),
    }
}

fn find_char_boundary(s: &str, i: usize) -> usize {
    for j in -4..4 {
        let k = i as i64 + j;
        if k < 0 || k >= s.len() as i64 {
            continue;
        }
        if s.is_char_boundary(k as usize) {
            return k as usize;
        }
    }

    panic!("char boundary not found");
}

struct ReplayProcess(Child);

impl Drop for ReplayProcess {
    fn drop(&mut self) {
        // A failed assertion or response timeout must not leave a server running.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn replay_output(log_file: &Path) -> Vec<lsp::Message> {
    let recorded = messages(handle_io(std::fs::read(log_file)));
    let mut recorded_replies = HashMap::new();
    let mut inputs = Vec::new();
    for message in recorded {
        if let lsp::Message::Response(response) = message {
            recorded_replies.insert(response.id.clone(), response);
        } else {
            inputs.push(message);
        }
    }

    let mut child = ReplayProcess(handle_io(
        cli()
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn(),
    ));
    let mut stdin = child.0.stdin.take().unwrap();
    let mut stdout = io::BufReader::new(child.0.stdout.take().unwrap());
    let mut stderr = child.0.stderr.take().unwrap();
    let (send, receive) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        while let Some(message) = handle_io(lsp::Message::read(&mut stdout)) {
            if send.send(message).is_err() {
                break;
            }
        }
    });
    let errors = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        handle_io(stderr.read_to_end(&mut bytes));
        bytes
    });
    let mut output = Vec::new();
    let mut transient_errors = Vec::new();
    for message in inputs {
        let request = match &message {
            lsp::Message::Request(request) => Some(request.clone()),
            _ => None,
        };
        handle_io(message.write(&mut stdin));
        let Some(request) = request else {
            continue;
        };
        // Raw --replay floods edits without their original timing. Obsolete
        // queued analyses then legitimately return ContentModified depending on
        // worker scheduling. This content snapshot test finishes each request
        // before the next recorded operation. Delayed filesystem updates can
        // still invalidate a snapshot, so retry that specific error with the
        // same parameters, at most five times. Keep those responses separately;
        // cancellation and overlapping edits are tested elsewhere.
        let mut retries = 0;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .expect("server did not finish the recorded request");
            let message = receive
                .recv_timeout(remaining)
                .expect("server did not finish the recorded request");
            match &message {
                lsp::Message::Response(response) => {
                    assert_eq!(response.id, request.id, "unexpected response: {response:?}");
                    if response.error.as_ref().is_some_and(|error| {
                        error.code == sync_ls::ErrorCode::ContentModified as i32
                    }) {
                        retries += 1;
                        assert!(retries <= 5, "snapshot never stabilized: {request:?}");
                        transient_errors.push(message);
                        handle_io(lsp::Message::Request(request.clone()).write(&mut stdin));
                        continue;
                    }
                    output.push(message);
                    break;
                }
                lsp::Message::Request(server_request) => {
                    // A recorded client reply may occur later in the file. Send
                    // it as soon as requested so a response barrier cannot
                    // deadlock a server request awaiting that reply.
                    if let Some(reply) = recorded_replies.remove(&server_request.id) {
                        handle_io(lsp::Message::Response(reply).write(&mut stdin));
                    }
                }
                lsp::Message::Notification(_) => {}
            }
            output.push(message);
        }
    }
    handle_io(std::fs::write(
        log_file.with_file_name("transient_errors.json"),
        serde_json::to_vec_pretty(&transient_errors).unwrap(),
    ));
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = handle_io(child.0.try_wait()) {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "replay server did not exit after EOF"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    reader.join().unwrap();
    output.extend(receive);
    let err = errors.join().unwrap();
    // if contains panic
    let err = std::str::from_utf8(&err).unwrap();
    let panic = err.find("panic");
    if let Some(p) = panic {
        // capture surrounding lines
        let panic_prev = p.saturating_sub(1024);
        let panic_next = (p + 10240).min(err.len());
        // find char boundary
        let panic_prev = find_char_boundary(err, panic_prev);
        let panic_next = find_char_boundary(err, panic_next);

        panic!(
            "panic found in stderr logging: PANIC_BEGIN\n\n{}\n\nPANIC_END",
            &err[panic_prev..panic_next]
        );
    }

    assert!(status.success(), "replay server failed: {status}\n{err}");
    output
}

struct ReplayBuilder {
    id: i32,
    messages: Vec<lsp::Message>,
}

impl ReplayBuilder {
    fn request_(&mut self, method: String, req: Value) {
        let id = RequestId::from(self.id);
        self.id += 1;
        self.messages
            .push(lsp::Message::Request(lsp::Request::new(id, method, req)));
    }

    fn request<R: lsp_types::request::Request>(&mut self, req: Value) {
        self.request_(R::METHOD.to_owned(), req);
    }

    fn notify_(&mut self, method: String, params: Value) {
        self.messages
            .push(lsp::Message::Notification(lsp::Notification::new(
                method, params,
            )));
    }

    fn notify<N: lsp_types::notification::Notification>(&mut self, params: Value) {
        self.notify_(N::METHOD.to_owned(), params);
    }
}

fn fixture(o: &str, f: impl FnOnce(&mut Value)) -> Value {
    // tests/fixtures/o.json
    let content = std::fs::read_to_string(format!("tests/fixtures/{o}.json")).unwrap();
    let mut req = serde_json::from_str(&content).unwrap();
    f(&mut req);
    req
}

fn gen(root: &Path, f: impl FnOnce(&mut ReplayBuilder)) {
    let mut builder = ReplayBuilder {
        id: 1,
        messages: Vec::new(),
    };
    f(&mut builder);
    // mkdir
    handle_io(std::fs::create_dir_all(root));
    // open root/mirror.log
    let mut log = std::fs::File::create(root.join("mirror.log")).unwrap();
    for msg in builder.messages {
        msg.write(&mut log).unwrap();
    }
}

fn messages(output: Vec<u8>) -> Vec<lsp::Message> {
    let mut output = std::io::BufReader::new(output.as_slice());
    // read all messages
    let mut messages = Vec::new();
    while let Ok(Some(msg)) = lsp::Message::read(&mut output) {
        // match msg
        messages.push(msg);
    }
    messages
}

struct SmokeArgs {
    root: PathBuf,
    init: String,
    log: String,
}

fn gen_smoke(args: SmokeArgs) {
    use lsp_types::notification::*;
    use lsp_types::request::*;
    use lsp_types::*;

    let SmokeArgs { root, init, log } = args;
    gen(&root, |srv| {
        let root_uri = lsp_types::Url::from_directory_path(&root).unwrap();
        srv.request::<Initialize>(fixture(&init, |v| {
            v["rootUri"] = json!(root_uri);
            v["rootPath"] = json!(root);
            v["workspaceFolders"] = json!([{
                "uri": root_uri,
                "name": "tinymist",
            }]);
        }));
        srv.notify::<Initialized>(json!({}));

        // open editions/base.log and readlines
        let log = std::fs::read_to_string(&log).unwrap();
        let log = log.trim().split('\n').collect::<Vec<_>>();
        let mut uri_set = HashSet::new();
        let mut uris = Vec::new();
        let log_lines = log.len();
        for (idx, line) in log.into_iter().enumerate() {
            let mut v: Value = serde_json::from_str(line).unwrap();

            // discover range in contentChanges and construct signatureHelp
            let mut range_seeds = vec![];
            if let Some(content_changes) = v
                .get_mut("params")
                .and_then(|v| v.get_mut("contentChanges"))
            {
                for change in content_changes.as_array_mut().unwrap() {
                    let range = change.get("range");
                    if let Some(range) = range {
                        let range: Range = serde_json::from_value(range.clone()).unwrap();
                        range_seeds.push(range);
                    }
                }
            }

            let uri_name = v["params"]["textDocument"]["uri"].as_str().unwrap();
            let url_v = if uri_name.starts_with("file:") || uri_name.starts_with("untitled:") {
                lsp_types::Url::parse(uri_name).unwrap()
            } else {
                root_uri.join(uri_name).unwrap()
            };
            v["params"]["textDocument"]["uri"] = json!(url_v);
            let method = v["method"].as_str().unwrap();
            srv.notify_("textDocument/".to_owned() + method, v["params"].clone());

            let mut request_at_loc = |loc: Position| {
                let pos = TextDocumentPositionParams {
                    text_document: TextDocumentIdentifier { uri: url_v.clone() },
                    position: loc,
                };
                srv.request::<SignatureHelpRequest>(json!(SignatureHelpParams {
                    context: None,
                    work_done_progress_params: Default::default(),
                    text_document_position_params: pos.clone(),
                }));
                srv.request::<HoverRequest>(json!(HoverParams {
                    work_done_progress_params: Default::default(),
                    text_document_position_params: pos.clone(),
                }));
                if log_lines == idx + 1 || log_lines == idx + 5 || log_lines == idx + 10 {
                    srv.request::<Completion>(json!(CompletionParams {
                        text_document_position: pos.clone(),
                        context: None,
                        work_done_progress_params: Default::default(),
                        partial_result_params: Default::default(),
                    }));
                }
                srv.request::<GotoDefinition>(json!(GotoDefinitionParams {
                    text_document_position_params: pos.clone(),
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                }));
                srv.request::<References>(json!(ReferenceParams {
                    text_document_position: pos.clone(),
                    context: ReferenceContext {
                        include_declaration: false,
                    },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default(),
                }));
            };

            let mut seed_at_loc = |loc: Position| {
                for i in loc.character.saturating_sub(2)..loc.character + 2 {
                    request_at_loc(Position {
                        line: loc.line,
                        character: i,
                    });
                }
                for l_delta in -1i32..1i32 {
                    if l_delta == 0 {
                        continue;
                    }
                    let l = (loc.line as i32) + l_delta;
                    if l < 0 {
                        continue;
                    }
                    let l = l as u32;
                    for i in 0..3 {
                        request_at_loc(Position {
                            line: l,
                            character: i,
                        });
                    }
                }

                // 10..100
                for l_delta in -20i32..20i32 {
                    if l_delta == 0 {
                        continue;
                    }
                    let l = (loc.line as i32) + l_delta * 5;
                    if l < 0 {
                        continue;
                    }
                    let l = l as u32;
                    request_at_loc(Position {
                        line: l,
                        character: 0,
                    });
                    request_at_loc(Position {
                        line: l,
                        character: 2,
                    });
                }
            };

            for r in range_seeds {
                seed_at_loc(r.start);
                seed_at_loc(r.end);
            }

            if uri_set.insert(url_v.clone()) {
                uris.push(url_v);
            }
            const MI_POS: Position = Position {
                line: 0,
                character: 0,
            };
            const MX_POS: Position = Position {
                line: u32::MAX / 1024,
                character: u32::MAX / 1024,
            };
            for u in &uris {
                srv.request::<FoldingRangeRequest>(json!(FoldingRangeParams {
                    text_document: TextDocumentIdentifier { uri: u.clone() },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default()
                }));
                srv.request::<DocumentSymbolRequest>(json!(DocumentSymbolParams {
                    text_document: TextDocumentIdentifier { uri: u.clone() },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default()
                }));
                srv.request::<CodeLensRequest>(json!(CodeLensParams {
                    text_document: TextDocumentIdentifier { uri: u.clone() },
                    work_done_progress_params: Default::default(),
                    partial_result_params: Default::default()
                }));
                srv.request::<InlayHintRequest>(json!(InlayHintParams {
                    text_document: TextDocumentIdentifier { uri: u.clone() },
                    work_done_progress_params: Default::default(),
                    range: Range {
                        start: MI_POS,
                        end: MX_POS
                    }
                }));

                if log_lines == idx + 1 {
                    srv.request::<SemanticTokensFullRequest>(json!(SemanticTokensParams {
                        text_document: TextDocumentIdentifier { uri: u.clone() },
                        work_done_progress_params: Default::default(),
                        partial_result_params: Default::default(),
                    }));
                }
            }
        }
    });
}

fn replay_log(root: &Path) -> String {
    let mut res = replay_output(&root.join("mirror.log"));
    // retain not notification
    res.retain(|msg| matches!(msg, lsp::Message::Response(_)));
    // sort by id
    res.sort_by_key(|msg| match msg {
        lsp::Message::Request(req) => req.id.clone(),
        lsp::Message::Response(res) => res.id.clone(),
        lsp::Message::Notification(_) => RequestId::from(0),
    });
    // Preserve the original responses separately from snapshot normalization.
    let raw = serde_json::to_string_pretty(&res).unwrap();
    std::fs::write(root.join("result.json"), raw).unwrap();
    // Token result IDs are opaque cache handles. A ContentModified retry can
    // consume an ID without changing the returned token data. Normalize handles
    // by first appearance, preserving repeated-handle relationships.
    let mut token_results = HashMap::new();
    for message in &mut res {
        let lsp::Message::Response(response) = message else {
            continue;
        };
        let Some(id) = response
            .result
            .as_mut()
            .and_then(|result| result.get_mut("resultId"))
        else {
            continue;
        };
        let next = (token_results.len() + 1).to_string();
        let normalized = token_results
            .entry(id.as_str().unwrap().to_owned())
            .or_insert(next);
        *id = json!(normalized);
    }
    let res = serde_json::to_value(&res).unwrap();
    // let sorted_res
    let sorted_res = sort_and_redact_value(res);
    let c = serde_json::to_string_pretty(&sorted_res).unwrap();
    let hash = tinymist_std::hash::hash128(&c);
    std::fs::write(root.join("result_sorted.json"), c).unwrap();

    format!("siphash128_13:{hash:x}")
}

fn sort_and_redact_value(v: Value) -> Value {
    match v {
        Value::Null => Value::Null,
        Value::Bool(b) => Value::Bool(b),
        Value::Number(n) => Value::Number(n),
        Value::String(s) => {
            if s.starts_with("file:") || s.starts_with("untitled:") {
                may_redact_uri(&s)
            } else {
                Value::String(s)
            }
        }
        Value::Array(a) => {
            let mut a = a;
            a.sort_by(json_cmp);
            Value::Array(a.into_iter().map(sort_and_redact_value).collect())
        }
        Value::Object(o) => {
            let mut keys = o.keys().collect::<Vec<_>>();
            keys.sort();
            Value::Object(
                keys.into_iter()
                    .map(|k| {
                        (k.clone(), {
                            let v = &o[k];
                            if k == "uri" || k == "targetUri" {
                                let uri = v.as_str().unwrap();
                                may_redact_uri(uri)
                            } else if k == "serverInfo" {
                                // Redact server info to avoid unstable version information
                                Value::Object(serde_json::Map::from_iter([
                                    ("name".to_string(), Value::String("tinymist".to_string())),
                                    (
                                        "version".to_string(),
                                        Value::String("<redacted>".to_string()),
                                    ),
                                ]))
                            } else {
                                sort_and_redact_value(v.clone())
                            }
                        })
                    })
                    .collect(),
            )
        }
    }
}

fn json_cmp(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => std::cmp::Ordering::Equal,
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::Number(a), Value::Number(b)) => {
            if let (Some(a), Some(b)) = (a.as_i64(), b.as_i64()) {
                a.cmp(&b)
            } else if let (Some(a), Some(b)) = (a.as_u64(), b.as_u64()) {
                a.cmp(&b)
            } else if let (Some(a), Some(b)) = (a.as_f64(), b.as_f64()) {
                a.partial_cmp(&b).unwrap()
            } else {
                panic!("unexpected number type");
            }
        }
        (Value::String(a), Value::String(b)) => a.cmp(b),
        (Value::Array(a), Value::Array(b)) => {
            let mut a = a.clone();
            let mut b = b.clone();
            if a.len() != b.len() {
                return a.len().cmp(&b.len());
            }

            a.sort_by(json_cmp);
            b.sort_by(json_cmp);
            for (a, b) in a.iter().zip(b.iter()) {
                let cmp = json_cmp(a, b);
                if cmp != std::cmp::Ordering::Equal {
                    return cmp;
                }
            }

            std::cmp::Ordering::Equal
        }
        (Value::Object(a), Value::Object(b)) => {
            let mut keys_a = a.keys().collect::<Vec<_>>();
            let mut keys_b = b.keys().collect::<Vec<_>>();
            keys_a.sort();
            keys_b.sort();
            if keys_a != keys_b {
                return keys_a.cmp(&keys_b);
            }
            for k in keys_a {
                let cmp = json_cmp(&a[k], &b[k]);
                if cmp != std::cmp::Ordering::Equal {
                    return cmp;
                }
            }
            std::cmp::Ordering::Equal
        }
        _ => std::cmp::Ordering::Equal,
    }
}

/// Gets uri and sets as file name
fn may_redact_uri(uri: &str) -> Value {
    if uri == "file://" || uri == "file:///" {
        Value::String("".to_owned())
    } else {
        let uri = lsp_types::Url::parse(uri).unwrap();

        match uri.to_file_path() {
            Ok(path) => {
                let path = path.file_name().unwrap().to_str().unwrap();
                Value::String(path.to_owned())
            }
            Err(_) => Value::String(uri.to_string()),
        }
    }
}
