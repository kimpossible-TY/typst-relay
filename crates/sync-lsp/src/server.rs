//! A synchronous language server implementation.

#[cfg(feature = "dap")]
mod dap_srv;

#[cfg(feature = "lsp")]
mod lsp_srv;

use core::fmt;
use std::any::Any;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::AtomicI32;
#[cfg(feature = "web")]
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Weak};

use futures::future::{AbortHandle, AbortRegistration, Abortable, MaybeDone};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Value as JsonValue, from_value};
use tinymist_std::time::Time;

use crate::msg::*;
use crate::req_queue;
use crate::*;

type ImmutPath = Arc<Path>;

/// A future that may be done in place or not.
pub type ResponseFuture<T> = MaybeDone<Pin<Box<dyn std::future::Future<Output = T> + Send>>>;
/// A future that may be rejected before actual started.
pub type LspResponseFuture<T> = LspResult<ResponseFuture<T>>;
/// A future that could be rejected by common error in `LspResponseFuture`.
pub type SchedulableResponse<T> = LspResponseFuture<LspResult<T>>;
/// The common response future type for language servers.
pub type AnySchedulableResponse = SchedulableResponse<JsonValue>;
/// The result of a scheduling response
pub type ScheduleResult = AnySchedulableResponse;
/// The result of a scheduled response which could be finally caught by
/// `schedule_tail`.
/// - Returns Ok(Some()) -> Already responded
/// - Returns Ok(None) -> Need to respond none
/// - Returns Err(..) -> Need to respond error
pub type ScheduledResult = LspResult<Option<()>>;

/// The untyped connect tx for language servers.
pub type ConnectionTx = TConnectionTx<Message>;
/// The untyped connect rx for language servers.
pub type ConnectionRx = TConnectionRx<Message>;

/// The sender of the language server.
#[derive(Debug, Clone)]
pub struct TConnectionTx<M> {
    /// The sender of the events.
    pub event: crossbeam_channel::Sender<Event>,
    /// The sender of the LSP messages.
    pub lsp: crossbeam_channel::Sender<Message>,
    pub(crate) marker: std::marker::PhantomData<M>,
}

/// The sender of the language server.
#[derive(Debug, Clone)]
pub struct TConnectionRx<M> {
    /// The receiver of the events.
    pub event: crossbeam_channel::Receiver<Event>,
    /// The receiver of the LSP messages.
    pub lsp: crossbeam_channel::Receiver<Message>,
    pub(crate) marker: std::marker::PhantomData<M>,
}

impl<M: TryFrom<Message, Error = anyhow::Error>> TConnectionRx<M> {
    /// Receives a message or an event.
    pub fn recv(&self) -> anyhow::Result<EventOrMessage<M>> {
        crossbeam_channel::select_biased! {
            recv(self.lsp) -> msg => Ok(EventOrMessage::Msg(msg?.try_into()?)),
            recv(self.event) -> event => Ok(event.map(EventOrMessage::Evt)?),
        }
    }
}

/// This is a helper enum to handle both events and messages.
pub enum EventOrMessage<M> {
    /// An event received.
    Evt(Event),
    /// A message received.
    Msg(M),
}

/// Connection is just a pair of channels of LSP messages.
pub struct Connection<M> {
    /// The senders of the connection.
    pub sender: TConnectionTx<M>,
    /// The receivers of the connection.
    pub receiver: TConnectionRx<M>,
}

impl<M> Connection<M> {
    /// Creates an in-memory connection backed by channels.
    pub fn channel() -> Self {
        let (event_sender, event_receiver) = crossbeam_channel::unbounded::<crate::Event>();
        let (lsp_sender, lsp_receiver) = crossbeam_channel::unbounded::<Message>();
        Self {
            sender: TConnectionTx {
                event: event_sender,
                lsp: lsp_sender,
                marker: std::marker::PhantomData,
            },
            receiver: TConnectionRx {
                event: event_receiver,
                lsp: lsp_receiver,
                marker: std::marker::PhantomData,
            },
        }
    }
}

impl<M: TryFrom<Message, Error = anyhow::Error>> From<Connection<Message>> for Connection<M> {
    fn from(conn: Connection<Message>) -> Self {
        Self {
            sender: TConnectionTx {
                event: conn.sender.event,
                lsp: conn.sender.lsp,
                marker: std::marker::PhantomData,
            },
            receiver: TConnectionRx {
                event: conn.receiver.event,
                lsp: conn.receiver.lsp,
                marker: std::marker::PhantomData,
            },
        }
    }
}

impl<M: TryFrom<Message, Error = anyhow::Error>> From<TConnectionTx<M>> for ConnectionTx {
    fn from(conn: TConnectionTx<M>) -> Self {
        Self {
            event: conn.event,
            lsp: conn.lsp,
            marker: std::marker::PhantomData,
        }
    }
}

type AnyCaster<S> = Arc<dyn Fn(&mut dyn Any) -> &mut S + Send + Sync>;

/// A Lsp client with typed service `S`.
pub struct TypedLspClient<S> {
    client: LspClient,
    caster: AnyCaster<S>,
}

impl<S> TypedLspClient<S> {
    /// Converts the client to an untyped client.
    pub fn to_untyped(self) -> LspClient {
        self.client
    }
}

impl<S: 'static> TypedLspClient<S> {
    /// Returns the untyped lsp client.
    pub fn untyped(&self) -> &LspClient {
        &self.client
    }

    /// Casts the service to another type.
    pub fn cast<T: 'static>(&self, f: fn(&mut S) -> &mut T) -> TypedLspClient<T> {
        let caster = self.caster.clone();
        TypedLspClient {
            client: self.client.clone(),
            caster: Arc::new(move |s| f(caster(s))),
        }
    }

    /// Sends a event to the client itself.
    pub fn send_event<T: std::any::Any + Send + 'static>(&self, event: T) {
        self.sender.send_event(event);
    }
}

impl<S> Clone for TypedLspClient<S> {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
            caster: self.caster.clone(),
        }
    }
}

impl<S> std::ops::Deref for TypedLspClient<S> {
    type Target = LspClient;

    fn deref(&self) -> &Self::Target {
        &self.client
    }
}

// send_request: Function,
// send_notification: Function,
/// The root of the language server host.
/// Will close connection when dropped.
#[derive(Debug, Clone)]
pub struct LspClientRoot {
    weak: LspClient,
    _strong: Arc<ConnectionTx>,
}

impl LspClientRoot {
    /// Creates a new language server host.
    pub fn new<M: TryFrom<Message, Error = anyhow::Error> + GetMessageKind>(
        handle: tokio::runtime::Handle,
        sender: TConnectionTx<M>,
    ) -> Self {
        let _strong = Arc::new(sender.into());
        let weak = LspClient {
            handle,
            msg_kind: M::MESSAGE_KIND,
            sender: TransportHost::System(SystemTransportSender {
                sender: Arc::downgrade(&_strong),
            }),
            req_queue: Arc::new(Mutex::new(ReqQueue::default())),

            hook: Arc::new(()),
        };
        Self { weak, _strong }
    }

    /// Creates a new language server host from js.
    #[cfg(feature = "web")]
    pub fn new_js(handle: tokio::runtime::Handle, sender: JsTransportSender) -> Self {
        let dummy = dummy_transport::<LspMessage>();

        let _strong = Arc::new(dummy.sender.into());
        let weak = LspClient {
            handle,
            msg_kind: LspMessage::MESSAGE_KIND,
            sender: TransportHost::Js {
                event_id: Arc::new(AtomicU32::new(0)),
                events: Arc::new(Mutex::new(HashMap::new())),
                sender,
            },
            req_queue: Arc::new(Mutex::new(ReqQueue::default())),

            hook: Arc::new(()),
        };
        Self { weak, _strong }
    }

    /// Sets the hook for the language server host.
    pub fn with_hook(mut self, hook: Arc<dyn LsHook>) -> Self {
        self.weak.hook = hook;
        self
    }

    /// Returns the weak reference to the language server host.
    pub fn weak(&self) -> LspClient {
        self.weak.clone()
    }
}

type ReqHandler = Box<dyn for<'a> FnOnce(&'a mut dyn Any, LspOrDapResponse) + Send + Sync>;
#[derive(Debug)]
pub(crate) struct PendingRequest {
    method: String,
    received_at: Time,
    identity: Arc<()>,
    abort: AbortHandle,
    registration: Option<AbortRegistration>,
}

type ReqQueue = req_queue::ReqQueue<PendingRequest, ReqHandler>;

/// Different transport mechanisms for communication.
#[derive(Debug, Clone)]
pub enum TransportHost {
    /// System-level transport using native OS capabilities.
    System(SystemTransportSender),
    /// JavaScript/WebAssembly transport for web environments.
    #[cfg(feature = "web")]
    Js {
        /// Atomic counter for generating unique event identifiers.
        event_id: Arc<AtomicU32>,
        /// Thread-safe storage for pending events indexed by their IDs.
        events: Arc<Mutex<HashMap<u32, Event>>>,
        /// The actual sender implementation for JavaScript environments.
        sender: JsTransportSender,
    },
}

/// A sender implementation for system-level transport operations.
#[derive(Debug, Clone)]
pub struct SystemTransportSender {
    /// Weak reference to the connection transmitter.
    pub(crate) sender: Weak<ConnectionTx>,
}

/// Creates a new js transport host.
#[cfg(feature = "web")]
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsTransportSender {
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub(crate) send_event: js_sys::Function,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub(crate) send_request: js_sys::Function,
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub(crate) send_notification: js_sys::Function,
    /// The acutal resolving function in JavaScript
    #[serde(with = "serde_wasm_bindgen::preserve")]
    pub resolve_fn: js_sys::Function,
}

#[cfg(feature = "web")]
/// SAFETY:
/// This is only safe if the `JsTransportHost` is used in a single thread.
unsafe impl Send for TransportHost {}

#[cfg(feature = "web")]
/// SAFETY:
/// This is only safe if the `JsTransportHost` is used in a single thread.
unsafe impl Sync for TransportHost {}

impl TransportHost {
    /// Sends a event to the server itself.
    pub fn send_event<T: std::any::Any + Send + 'static>(&self, event: T) {
        match self {
            TransportHost::System(host) => {
                let Some(sender) = host.sender.upgrade() else {
                    log::warn!("failed to send request: connection closed");
                    return;
                };

                if let Err(res) = sender.event.send(Box::new(event)) {
                    log::warn!("failed to send event: {res:?}");
                }
            }
            #[cfg(feature = "web")]
            TransportHost::Js {
                event_id,
                sender,
                events,
            } => {
                let event_id = {
                    let event_id = event_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let mut lg = events.lock();
                    lg.insert(event_id, Box::new(event));
                    js_sys::Number::from(event_id)
                };
                if let Err(err) = sender
                    .send_event
                    .call1(&wasm_bindgen::JsValue::UNDEFINED, &event_id.into())
                {
                    log::error!("failed to send event: {err:?}");
                }
            }
        }
    }

    /// Sends a message.
    pub fn send_message(&self, response: Message) {
        match self {
            TransportHost::System(host) => {
                let Some(sender) = host.sender.upgrade() else {
                    log::warn!("failed to send response: connection closed");
                    return;
                };
                if let Err(res) = sender.lsp.send(response) {
                    log::warn!("failed to send response: {res:?}");
                }
            }
            #[cfg(feature = "web")]
            TransportHost::Js { sender, .. } => match response {
                #[cfg(feature = "lsp")]
                Message::Lsp(lsp::Message::Request(req)) => {
                    let msg = to_js_value(&req).expect("failed to serialize request to js value");
                    if let Err(err) = sender
                        .send_request
                        .call1(&wasm_bindgen::JsValue::UNDEFINED, &msg)
                    {
                        log::error!("failed to send request: {err:?}");
                    }
                }
                #[cfg(feature = "lsp")]
                Message::Lsp(lsp::Message::Notification(req)) => {
                    let msg = to_js_value(&req).expect("failed to serialize request to js value");
                    if let Err(err) = sender
                        .send_notification
                        .call1(&wasm_bindgen::JsValue::UNDEFINED, &msg)
                    {
                        log::error!("failed to send request: {err:?}");
                    }
                }
                #[cfg(feature = "lsp")]
                Message::Lsp(lsp::Message::Response(req)) => {
                    panic!("unexpected response to js world: {req:?}");
                }
                #[cfg(feature = "dap")]
                Message::Dap(dap::Message::Request(req)) => {
                    let msg = to_js_value(&req).expect("failed to serialize request to js value");
                    if let Err(err) = sender
                        .send_request
                        .call1(&wasm_bindgen::JsValue::UNDEFINED, &msg)
                    {
                        log::error!("failed to send request: {err:?}");
                    }
                }
                #[cfg(feature = "dap")]
                Message::Dap(dap::Message::Event(req)) => {
                    let msg = to_js_value(&req).expect("failed to serialize request to js value");
                    if let Err(err) = sender
                        .send_notification
                        .call1(&wasm_bindgen::JsValue::UNDEFINED, &msg)
                    {
                        log::error!("failed to send request: {err:?}");
                    }
                }
                #[cfg(feature = "dap")]
                Message::Dap(dap::Message::Response(req)) => {
                    panic!("unexpected response to js world: {req:?}");
                }
                #[cfg(feature = "dap")]
                Message::Dap(dap::Message::ResponseWithCommand(req)) => {
                    panic!("unexpected response to js world: {req:?}");
                }
            },
        }
    }
}

// todo: poor performance, struct -> serde_json -> serde_wasm_bindgen ->
// serialize -> deserialize??
#[cfg(feature = "web")]
fn to_js_value<T: serde::Serialize>(
    value: &T,
) -> Result<wasm_bindgen::JsValue, serde_wasm_bindgen::Error> {
    value.serialize(&serde_wasm_bindgen::Serializer::new().serialize_maps_as_objects(true))
}

/// The host for the language server, or known as the LSP client.
#[derive(Debug, Clone)]
pub struct LspClient {
    /// The tokio handle.
    pub handle: tokio::runtime::Handle,

    pub(crate) msg_kind: MessageKind,
    /// The TransportHost between LspClient and LspServer
    pub sender: TransportHost,
    pub(crate) req_queue: Arc<Mutex<ReqQueue>>,

    pub(crate) hook: Arc<dyn LsHook>,
}

impl LspClient {
    /// Returns the untyped lsp client.
    pub fn untyped(&self) -> &Self {
        self
    }

    /// converts the client to a typed client.
    pub fn to_typed<S: Any>(&self) -> TypedLspClient<S> {
        TypedLspClient {
            client: self.clone(),
            caster: Arc::new(|s| s.downcast_mut().expect("invalid cast")),
        }
    }

    /// Checks if there are pending requests.
    pub fn has_pending_requests(&self) -> bool {
        self.req_queue.lock().incoming.has_pending()
    }

    /// Prints states of the request queue and panics.
    pub fn begin_panic(&self) {
        self.req_queue.lock().begin_panic();
    }

    /// Sends a event to the server itself.
    pub fn send_event<T: std::any::Any + Send + 'static>(&self, event: T) {
        self.sender.send_event(event);
    }

    /// Completes an server2client request in the request queue.
    #[cfg(feature = "lsp")]
    pub fn complete_lsp_request<S: Any>(&self, service: &mut S, response: lsp::Response) {
        let mut req_queue = self.req_queue.lock();
        let Some(handler) = req_queue.outgoing.complete(response.id.clone()) else {
            log::warn!("received response for unknown request");
            return;
        };
        drop(req_queue);
        handler(service, response.into())
    }

    /// Completes an server2client request in the request queue.
    #[cfg(feature = "dap")]
    pub fn complete_dap_request<S: Any>(&self, service: &mut S, response: dap::Response) {
        let mut req_queue = self.req_queue.lock();
        let Some(handler) = req_queue
            .outgoing
            // todo: casting i64 to i32
            .complete((response.request_seq as i32).into())
        else {
            log::warn!("received response for unknown request");
            return;
        };
        drop(req_queue);
        handler(service, response.into())
    }

    /// Registers an client2server request in the request queue.
    pub fn register_request(&self, method: &str, id: &RequestId, received_at: Time) {
        let mut req_queue = self.req_queue.lock();
        self.hook.start_request(id, method);
        let (abort, registration) = AbortHandle::new_pair();
        req_queue.incoming.register(
            id.clone(),
            PendingRequest {
                method: method.to_owned(),
                received_at,
                identity: Arc::new(()),
                abort,
                registration: Some(registration),
            },
        );
    }

    /// Cancels a pending read-only query and sends its only terminal response.
    ///
    /// Dropping the response future also drops queued work. Synchronous work
    /// that has already started must finish safely; its eventual result is
    /// ignored because the request is no longer pending.
    ///
    /// Lifecycle, execute-command, and unknown extension requests are not
    /// cancellation-safe by default: their synchronous handlers can change
    /// state before returning a future that must finish initialization or
    /// cleanup. Ignore cancellation for those until they explicitly support it.
    #[cfg(feature = "lsp")]
    pub fn cancel_request(&self, id: RequestId) {
        let pending = self
            .req_queue
            .lock()
            .incoming
            .get_mut(&id)
            .filter(|r| {
                r.method.starts_with("textDocument/")
                    || matches!(
                        r.method.as_str(),
                        "workspace/symbol" | "workspace/willRenameFiles"
                    )
            })
            .map(|r| (r.abort.clone(), r.identity.clone()));
        let Some((abort, identity)) = pending else {
            return;
        };
        abort.abort();
        self.respond_result(
            id,
            &identity,
            Err(ResponseError {
                code: ErrorCode::RequestCanceled as i32,
                message: "cancelled by client".into(),
                data: None,
            }),
        );
    }

    fn respond_result(&self, id: RequestId, identity: &Arc<()>, result: LspResult<JsonValue>) {
        let req_id = id.clone();
        let msg: Message = match (self.msg_kind, result) {
            #[cfg(feature = "lsp")]
            (MessageKind::Lsp, res) => lsp::Response::new(id, res).into(),
            #[cfg(feature = "dap")]
            (MessageKind::Dap, Ok(resp)) => dap::Response::success(RequestId::dap(id), resp).into(),
            #[cfg(feature = "dap")]
            (MessageKind::Dap, Err(e)) => {
                dap::Response::error(RequestId::dap(id), Some(e.message), None).into()
            }
        };

        self.respond_if_current(req_id, msg, Some(identity));
    }

    /// Completes an client2server request in the request queue.
    pub fn respond(&self, id: RequestId, response: Message) {
        self.respond_if_current(id, response, None);
    }

    fn respond_if_current(&self, id: RequestId, response: Message, identity: Option<&Arc<()>>) {
        let mut req_queue = self.req_queue.lock();
        if let Some(identity) = identity {
            let current = req_queue.incoming.get_mut(&id);
            if !current.is_some_and(|request| Arc::ptr_eq(&request.identity, identity)) {
                return;
            }
        }
        let Some(PendingRequest {
            method,
            received_at,
            abort,
            ..
        }) = req_queue.incoming.complete(&id)
        else {
            return;
        };

        self.hook.stop_request(&id, &method, received_at);

        let delay = tinymist_std::time::now().duration_since(received_at);
        match delay {
            Ok(delay) => {
                if delay.as_secs() > 10 {
                    let worst_outgoing = req_queue
                        .incoming
                        .pending()
                        .max_by_key(|(_, data)| data.received_at);
                    let worst_case = if let Some((id, pending)) = worst_outgoing {
                        let method = &pending.method;
                        let duration =
                            tinymist_std::time::now().duration_since(pending.received_at);
                        format!(", worst case: req({method:?}, {id:?}) - {duration:?}")
                    } else {
                        String::new()
                    };
                    log::warn!(
                        "request {id:?} is completed after {delay:?}, pending incoming requests: {:?}, pending outgoing requests: {:?}{worst_case}",
                        req_queue.incoming,
                        req_queue.outgoing
                    );
                }
            }
            Err(err) => {
                log::error!("failed to get delay for request {id:?}: {err:?}");
            }
        }

        #[cfg(feature = "dap")]
        let response = match response {
            Message::Dap(dap::Message::Response(resp)) => Message::Dap(
                dap::Message::ResponseWithCommand(dap::ResponseWithCommand::new(method, resp)),
            ),
            response => response,
        };

        drop(req_queue);
        // A direct response can complete a request whose scheduled future is
        // still waiting. Wake it after releasing the queue lock, on every
        // protocol including DAP.
        abort.abort();
        self.sender.send_message(response);
    }
}

impl LspClient {
    /// Finally sends the response if it is not sent before.
    /// From the definition, the response is already sent if it is `Some(())`.
    pub fn schedule_tail(
        self,
        req_id: RequestId,
        resp: ScheduleResult,
    ) -> impl std::future::Future<Output = ()> + Send + 'static {
        // Capture the registration before dispatch, not when the future first
        // polls: the client may cancel and then reuse this ID in the meantime.
        let pending = self
            .req_queue
            .lock()
            .incoming
            .get_mut(&req_id)
            .map(|request| (request.identity.clone(), request.registration.take()));
        async move {
            let Some((identity, registration)) = pending else {
                return;
            };
            match resp {
                Ok(MaybeDone::Done(result)) => {
                    self.respond_result(req_id, &identity, result);
                }
                Ok(MaybeDone::Future(result)) => {
                    if let Some(registration) = registration
                        && let Ok(result) = Abortable::new(result, registration).await
                    {
                        self.respond_result(req_id, &identity, result);
                    }
                }
                Ok(MaybeDone::Gone) => {
                    log::debug!("response for request({req_id:?}) was already sent");
                }
                Err(err) => {
                    self.respond_result(req_id, &identity, Err(err));
                }
            }
        }
    }
}

#[cfg(all(test, feature = "lsp", feature = "system"))]
mod cancellation_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn cancelled_requests_respond_once_and_never_start() {
        for id in [
            RequestId::from(7),
            RequestId::from("request-seven".to_owned()),
        ] {
            let connection = Connection::<LspMessage>::channel();
            let root = LspClientRoot::new(tokio::runtime::Handle::current(), connection.sender);
            let client = root.weak();
            let started = Arc::new(AtomicBool::new(false));
            let observed = started.clone();
            client.register_request("textDocument/completion", &id, tinymist_std::time::now());
            let response = just_future(async move {
                observed.store(true, Ordering::Release);
                Ok(JsonValue::Null)
            });
            client.cancel_request(id.clone());
            client.cancel_request(id.clone());
            client.clone().schedule_tail(id.clone(), response).await;
            let Message::Lsp(lsp::Message::Response(response)) =
                connection.receiver.lsp.try_recv().unwrap()
            else {
                panic!("expected cancellation response");
            };
            assert_eq!(response.id, id);
            assert_eq!(
                response.error.unwrap().code,
                ErrorCode::RequestCanceled as i32
            );
            assert!(!started.load(Ordering::Acquire));
            assert!(!client.has_pending_requests());
            assert!(connection.receiver.lsp.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn cancellation_drops_a_waiting_future() {
        let connection = Connection::<LspMessage>::channel();
        let root = LspClientRoot::new(tokio::runtime::Handle::current(), connection.sender);
        let client = root.weak();
        let id = RequestId::from(1);
        client.register_request("textDocument/hover", &id, tinymist_std::time::now());
        let (started, ready) = tokio::sync::oneshot::channel();
        let (retained, released) = tokio::sync::oneshot::channel::<()>();
        let response = just_future(async move {
            let _retained = retained;
            started.send(()).unwrap();
            futures::future::pending::<LspResult<JsonValue>>().await
        });
        let task = tokio::spawn(client.clone().schedule_tail(id.clone(), response));
        ready.await.unwrap();
        client.cancel_request(id);
        task.await.unwrap();
        assert!(released.await.is_err());
        assert!(!client.has_pending_requests());
        assert!(connection.receiver.lsp.try_recv().is_ok());
        assert!(connection.receiver.lsp.try_recv().is_err());
    }

    #[tokio::test]
    async fn late_and_unknown_cancellation_are_ignored() {
        let connection = Connection::<LspMessage>::channel();
        let root = LspClientRoot::new(tokio::runtime::Handle::current(), connection.sender);
        let client = root.weak();
        let id = RequestId::from(1);
        client.register_request("test", &id, tinymist_std::time::now());
        client
            .clone()
            .schedule_tail(id.clone(), just_ok(JsonValue::Null))
            .await;
        client.cancel_request(id);
        client.cancel_request(RequestId::from(999));
        let Message::Lsp(lsp::Message::Response(response)) =
            connection.receiver.lsp.try_recv().unwrap()
        else {
            panic!("expected successful response");
        };
        assert!(response.error.is_none());
        assert!(connection.receiver.lsp.try_recv().is_err());
    }

    #[tokio::test]
    async fn cancelled_future_cannot_take_a_reused_ids_registration() {
        for id in [RequestId::from(7), RequestId::from("reused".to_owned())] {
            let connection = Connection::<LspMessage>::channel();
            let root = LspClientRoot::new(tokio::runtime::Handle::current(), connection.sender);
            let client = root.weak();
            let started = Arc::new(AtomicBool::new(false));
            let observed = started.clone();

            client.register_request("textDocument/completion", &id, tinymist_std::time::now());
            let old = client.clone().schedule_tail(
                id.clone(),
                just_future(async move {
                    observed.store(true, Ordering::Release);
                    Ok(JsonValue::String("old".into()))
                }),
            );
            // The old future has not been polled when cancellation completes.
            client.cancel_request(id.clone());
            assert!(connection.receiver.lsp.try_recv().is_ok());

            client.register_request("textDocument/hover", &id, tinymist_std::time::now());
            let new = client.clone().schedule_tail(
                id.clone(),
                just_future(async { Ok(JsonValue::String("new".into())) }),
            );
            old.await;
            assert!(!started.load(Ordering::Acquire));
            assert!(client.has_pending_requests());
            assert!(connection.receiver.lsp.try_recv().is_err());

            new.await;
            let Message::Lsp(lsp::Message::Response(response)) =
                connection.receiver.lsp.try_recv().unwrap()
            else {
                panic!("expected the new request's response");
            };
            assert_eq!(response.id, id);
            assert_eq!(response.result, Some(JsonValue::String("new".into())));
            assert!(!client.has_pending_requests());
            assert!(connection.receiver.lsp.try_recv().is_err());
        }
    }

    #[tokio::test]
    async fn cancelled_ready_response_cannot_complete_a_reused_id() {
        let connection = Connection::<LspMessage>::channel();
        let root = LspClientRoot::new(tokio::runtime::Handle::current(), connection.sender);
        let client = root.weak();
        let id = RequestId::from(1);
        client.register_request("textDocument/completion", &id, tinymist_std::time::now());
        let old = client
            .clone()
            .schedule_tail(id.clone(), just_ok(JsonValue::String("old".into())));
        client.cancel_request(id.clone());
        assert!(connection.receiver.lsp.try_recv().is_ok());

        client.register_request("textDocument/hover", &id, tinymist_std::time::now());
        old.await;
        assert!(client.has_pending_requests());
        assert!(connection.receiver.lsp.try_recv().is_err());

        client
            .clone()
            .schedule_tail(id, just_ok(JsonValue::String("new".into())))
            .await;
        let Message::Lsp(lsp::Message::Response(response)) =
            connection.receiver.lsp.try_recv().unwrap()
        else {
            panic!("expected the new request's response");
        };
        assert_eq!(response.result, Some(JsonValue::String("new".into())));
        assert!(!client.has_pending_requests());
        assert!(connection.receiver.lsp.try_recv().is_err());
    }

    #[tokio::test]
    async fn stateful_requests_finish_before_responding_despite_cancellation() {
        for method in [
            "workspace/executeCommand",
            "initialize",
            "shutdown",
            "tinymist/customStateful",
        ] {
            let connection = Connection::<LspMessage>::channel();
            let root = LspClientRoot::new(tokio::runtime::Handle::current(), connection.sender);
            let client = root.weak();
            let id = RequestId::from(1);
            let finished = Arc::new(AtomicBool::new(false));
            let observed = finished.clone();
            let (started, ready) = tokio::sync::oneshot::channel();
            let (release, wait) = tokio::sync::oneshot::channel();

            client.register_request(method, &id, tinymist_std::time::now());
            let response = just_future(async move {
                started.send(()).unwrap();
                wait.await.unwrap();
                observed.store(true, Ordering::Release);
                Ok(JsonValue::Null)
            });
            let task = tokio::spawn(client.clone().schedule_tail(id.clone(), response));
            ready.await.unwrap();
            client.cancel_request(id.clone());
            assert!(client.has_pending_requests());
            assert!(connection.receiver.lsp.try_recv().is_err());
            assert!(!finished.load(Ordering::Acquire));

            release.send(()).unwrap();
            task.await.unwrap();
            assert!(finished.load(Ordering::Acquire));
            let Message::Lsp(lsp::Message::Response(response)) =
                connection.receiver.lsp.try_recv().unwrap()
            else {
                panic!("expected the completed stateful request's response");
            };
            assert_eq!(response.id, id);
            assert!(response.error.is_none());
            assert!(!client.has_pending_requests());
            assert!(connection.receiver.lsp.try_recv().is_err());
        }
    }
}

/// A trait that defines the hook for the language server.
pub trait LsHook: fmt::Debug + Send + Sync {
    /// Starts a request.
    fn start_request(&self, req_id: &RequestId, method: &str);
    /// Stops a request.
    fn stop_request(&self, req_id: &RequestId, method: &str, received_at: Time);
    /// Starts a notification.
    fn start_notification(&self, track_id: i32, method: &str);
    /// Stops a notification.
    fn stop_notification(
        &self,
        track_id: i32,
        method: &str,
        received_at: Time,
        result: LspResult<()>,
    );
}

impl LsHook for () {
    fn start_request(&self, req_id: &RequestId, method: &str) {
        log::info!("handling {method} - ({req_id})");
    }

    fn stop_request(&self, req_id: &RequestId, method: &str, received_at: Time) {
        let duration = received_at.elapsed();
        log::info!("handled  {method} - ({req_id}) in {duration:0.2?}");
    }

    fn start_notification(&self, track_id: i32, method: &str) {
        log::info!("notifying ({track_id}) - {method}");
    }

    fn stop_notification(
        &self,
        track_id: i32,
        method: &str,
        received_at: Time,
        result: LspResult<()>,
    ) {
        let request_duration = received_at.elapsed();
        if let Err(err) = result {
            log::error!(
                "notify ({track_id}) - {method} failed in {request_duration:0.2?}: {err:?}"
            );
        } else {
            log::info!("notify ({track_id}) - {method} succeeded in {request_duration:0.2?}");
        }
    }
}

type AsyncHandler<S, T, R> = fn(srv: &mut S, args: T) -> SchedulableResponse<R>;
type RawHandler<S, T> = fn(srv: &mut S, args: T) -> ScheduleResult;
type BoxPureHandler<S, T> = Box<dyn Fn(&mut S, T) -> LspResult<()>>;
type BoxHandler<S, T> = Box<dyn Fn(&mut S, RequestId, T) -> SchedulableResponse<JsonValue>>;
type ExecuteCmdMap<S> = HashMap<&'static str, BoxHandler<S, Vec<JsonValue>>>;
type RegularCmdMap<S> = HashMap<&'static str, BoxHandler<S, JsonValue>>;
type NotifyCmdMap<S> = HashMap<&'static str, BoxPureHandler<S, JsonValue>>;
type ResourceMap<S> = HashMap<ImmutPath, BoxHandler<S, Vec<JsonValue>>>;
type MayInitBoxHandler<A, S, T> =
    Box<dyn for<'a> Fn(ServiceState<'a, A, S>, &LspClient, T) -> anyhow::Result<()>>;
type EventMap<A, S> = HashMap<core::any::TypeId, MayInitBoxHandler<A, S, Event>>;

/// A trait that initializes the language server.
pub trait Initializer {
    /// The type of the initialization request.
    type I: for<'de> serde::Deserialize<'de>;
    /// The type of the service.
    type S;

    /// Handles the initialization request.
    /// If the behind protocol is the standard LSP, the request is
    /// `InitializeParams`.
    fn initialize(self, req: Self::I) -> (Self::S, AnySchedulableResponse);
}

/// The language server builder serving LSP.
#[cfg(feature = "lsp")]
pub type LspBuilder<Args> = LsBuilder<LspMessage, Args>;
/// The language server builder serving DAP.
#[cfg(feature = "dap")]
pub type DapBuilder<Args> = LsBuilder<DapMessage, Args>;

/// The builder pattern for the language server.
pub struct LsBuilder<M, Args: Initializer> {
    /// The extra initialization arguments.
    pub args: Args,
    /// The client surface for the implementing language server.
    pub client: LspClient,
    /// The event handlers.
    pub events: EventMap<Args, Args::S>,
    /// The command handlers.
    pub command_handlers: ExecuteCmdMap<Args::S>,
    /// The notification handlers.
    pub notif_handlers: NotifyCmdMap<Args::S>,
    /// The LSP request handlers.
    pub req_handlers: RegularCmdMap<Args::S>,
    /// The resource handlers.
    pub resource_handlers: ResourceMap<Args::S>,
    _marker: std::marker::PhantomData<M>,
}

impl<M, Args: Initializer> LsBuilder<M, Args>
where
    Args::S: 'static,
{
    /// Creates a new language server builder.
    pub fn new(args: Args, client: LspClient) -> Self {
        Self {
            args,
            client,
            events: EventMap::new(),
            command_handlers: ExecuteCmdMap::new(),
            notif_handlers: NotifyCmdMap::new(),
            req_handlers: RegularCmdMap::new(),
            resource_handlers: ResourceMap::new(),
            _marker: std::marker::PhantomData,
        }
    }

    /// Registers an event handler.
    pub fn with_event<T: std::any::Any>(
        mut self,
        ins: &T,
        handler: impl for<'a> Fn(ServiceState<'a, Args, Args::S>, T) -> anyhow::Result<()> + 'static,
    ) -> Self {
        self.events.insert(
            ins.type_id(),
            Box::new(move |s, _client, req| handler(s, *req.downcast().unwrap())),
        );
        self
    }

    /// Registers an async resource handler.
    pub fn with_resource(
        mut self,
        path: &'static str,
        handler: fn(&mut Args::S, Vec<JsonValue>) -> AnySchedulableResponse,
    ) -> Self {
        self.resource_handlers.insert(
            Path::new(path).into(),
            Box::new(move |s, _req_id, args| handler(s, args)),
        );
        self
    }

    /// Builds the language server driver.
    pub fn build(self) -> LsDriver<M, Args> {
        LsDriver {
            state: State::Uninitialized(Some(Box::new(self.args))),
            next_not_id: AtomicI32::new(1),
            events: self.events,
            client: self.client,
            commands: self.command_handlers,
            notifications: self.notif_handlers,
            requests: self.req_handlers,
            resources: self.resource_handlers,
            _marker: std::marker::PhantomData,
        }
    }
}

/// An enum to represent the state of the language server.
pub enum ServiceState<'a, A, S> {
    /// The service is uninitialized.
    Uninitialized(Option<&'a mut A>),
    /// The service is initializing.
    Ready(&'a mut S),
}

impl<A, S> ServiceState<'_, A, S> {
    /// Converts the state to an option holding the ready service.
    pub fn ready(&mut self) -> Option<&mut S> {
        match self {
            ServiceState::Ready(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State<Args, S> {
    Uninitialized(Option<Box<Args>>),
    Initializing(S),
    Ready(S),
    ShuttingDown,
}

impl<Args, S> State<Args, S> {
    fn opt(&self) -> Option<&S> {
        match &self {
            State::Ready(s) => Some(s),
            _ => None,
        }
    }

    fn opt_mut(&mut self) -> Option<&mut S> {
        match self {
            State::Ready(s) => Some(s),
            _ => None,
        }
    }
}

/// The language server driver.
pub struct LsDriver<M, Args: Initializer> {
    /// State to synchronize with the client.
    state: State<Args, Args::S>,
    /// The language server client.
    pub client: LspClient,
    /// The next notification ID.
    pub next_not_id: AtomicI32,

    // Handle maps
    /// Events for dispatching.
    pub events: EventMap<Args, Args::S>,
    /// Extra commands provided with `textDocument/executeCommand`.
    pub commands: ExecuteCmdMap<Args::S>,
    /// Notifications for dispatching.
    pub notifications: NotifyCmdMap<Args::S>,
    /// Requests for dispatching.
    pub requests: RegularCmdMap<Args::S>,
    /// Resources for dispatching.
    pub resources: ResourceMap<Args::S>,
    _marker: std::marker::PhantomData<M>,
}

impl<M, Args: Initializer> LsDriver<M, Args> {
    /// Gets the state of the language server.
    pub fn state(&self) -> Option<&Args::S> {
        self.state.opt()
    }

    /// Gets the mutable state of the language server.
    pub fn state_mut(&mut self) -> Option<&mut Args::S> {
        self.state.opt_mut()
    }

    /// Makes the language server ready.
    pub fn ready(&mut self, params: Args::I) -> AnySchedulableResponse {
        let args = match &mut self.state {
            State::Uninitialized(args) => args,
            _ => return just_result(Err(invalid_request("server is already initialized"))),
        };

        let args = args.take().expect("already initialized");
        let (s, res) = args.initialize(params);
        self.state = State::Ready(s);

        res
    }

    /// Get static resources with help of tinymist service, for example, a
    /// static help pages for some typst function.
    pub fn get_resources(&mut self, req_id: RequestId, args: Vec<JsonValue>) -> ScheduleResult {
        let s = self.state.opt_mut().ok_or_else(not_initialized)?;

        let path =
            from_value::<PathBuf>(args[0].clone()).map_err(|e| invalid_params(e.to_string()))?;

        let Some(handler) = self.resources.get(path.as_path()) else {
            log::error!("asked for unknown resource: {path:?}");
            return Err(method_not_found());
        };

        // Note our redirection will keep the first path argument in the args vec.
        handler(s, req_id, args)
    }
}

/// A helper function to create a `LspResponseFuture`
pub fn just_ok<T, E>(res: T) -> Result<ResponseFuture<Result<T, E>>, E> {
    Ok(futures::future::MaybeDone::Done(Ok(res)))
}

/// A helper function to create a `LspResponseFuture`
pub fn just_result<T, E>(res: Result<T, E>) -> Result<ResponseFuture<Result<T, E>>, E> {
    Ok(futures::future::MaybeDone::Done(res))
}

/// A helper function to create a `LspResponseFuture`
pub fn just_future<T, E>(
    fut: impl std::future::Future<Output = Result<T, E>> + Send + 'static,
) -> Result<ResponseFuture<Result<T, E>>, E> {
    Ok(futures::future::MaybeDone::Future(Box::pin(fut)))
}

/// Creates an invalid params error.
pub fn invalid_params(msg: impl fmt::Display) -> ResponseError {
    resp_err(ErrorCode::InvalidParams, msg)
}

/// Creates an internal error.
pub fn internal_error(msg: impl fmt::Display) -> ResponseError {
    resp_err(ErrorCode::InternalError, msg)
}

/// Creates a not initialized error.
pub fn not_initialized() -> ResponseError {
    resp_err(ErrorCode::ServerNotInitialized, "not initialized yet")
}

/// Creates a method not found error.
pub fn method_not_found() -> ResponseError {
    resp_err(ErrorCode::MethodNotFound, "method not found")
}

/// Creates an invalid request error.
pub fn invalid_request(msg: impl fmt::Display) -> ResponseError {
    resp_err(ErrorCode::InvalidRequest, msg)
}

fn from_json<T: serde::de::DeserializeOwned>(json: JsonValue) -> LspResult<T> {
    serde_json::from_value(json).map_err(invalid_request)
}

/// Erases the response type to a generic `JsonValue`.
pub fn erased_response<T: Serialize + 'static>(resp: SchedulableResponse<T>) -> ScheduleResult {
    /// Responds a typed result to the client.
    fn map_respond_result<T: Serialize>(result: LspResult<T>) -> LspResult<JsonValue> {
        result.and_then(|t| serde_json::to_value(t).map_err(internal_error))
    }

    let resp = resp?;

    use futures::future::MaybeDone::*;
    Ok(match resp {
        Done(result) => MaybeDone::Done(map_respond_result(result)),
        Future(fut) => MaybeDone::Future(Box::pin(async move { map_respond_result(fut.await) })),
        Gone => {
            log::warn!("response already taken");
            MaybeDone::Done(Err(internal_error("response already taken")))
        }
    })
}

/// Adapts a handler that may respond to the request itself.
pub fn scheduled_response(resp: ScheduledResult) -> ScheduleResult {
    match resp? {
        Some(()) => Ok(MaybeDone::Gone),
        None => just_ok(JsonValue::Null),
    }
}

fn resp_err(code: ErrorCode, msg: impl fmt::Display) -> ResponseError {
    ResponseError {
        code: code as i32,
        message: msg.to_string(),
        data: None,
    }
}
