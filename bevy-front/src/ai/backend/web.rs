use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{ErrorEvent, MessageEvent, Worker};

use super::BackendEvent;

const WORKER_URL: &str = "assets/engine/fairy-stockfish-client.worker.js";

pub(crate) struct Backend {
    worker: Worker,
    events: Rc<RefCell<VecDeque<BackendEvent>>>,
    _message_handler: Closure<dyn FnMut(MessageEvent)>,
    _error_handler: Closure<dyn FnMut(ErrorEvent)>,
}

impl Backend {
    pub(crate) fn new() -> Result<Self, String> {
        let worker = Worker::new(WORKER_URL).map_err(|error| {
            format!(
                "could not create the Fairy-Stockfish Web Worker: {}",
                js_error(&error)
            )
        })?;
        let events = Rc::new(RefCell::new(VecDeque::new()));

        let message_events = events.clone();
        let message_handler = Closure::new(move |event: MessageEvent| {
            if let Some(line) = event.data().as_string() {
                if let Some(message) = line.strip_prefix("__fairy_error__ ") {
                    message_events
                        .borrow_mut()
                        .push_back(BackendEvent::Error(message.to_owned()));
                } else {
                    message_events
                        .borrow_mut()
                        .push_back(BackendEvent::Line(line));
                }
            }
        });
        worker.set_onmessage(Some(message_handler.as_ref().unchecked_ref()));

        let error_events = events.clone();
        let error_handler = Closure::new(move |event: ErrorEvent| {
            error_events
                .borrow_mut()
                .push_back(BackendEvent::Error(format!(
                    "Fairy-Stockfish worker failed: {}. The server must enable COOP/COEP headers for WebAssembly threads.",
                    event.message()
                )));
        });
        worker.set_onerror(Some(error_handler.as_ref().unchecked_ref()));

        Ok(Self {
            worker,
            events,
            _message_handler: message_handler,
            _error_handler: error_handler,
        })
    }

    pub(crate) fn send(&self, command: &str) -> Result<(), String> {
        self.worker.post_message(&command.into()).map_err(|error| {
            format!(
                "could not send a command to Fairy-Stockfish: {}",
                js_error(&error)
            )
        })
    }

    pub(crate) fn drain(&self, destination: &mut Vec<BackendEvent>) {
        destination.extend(self.events.borrow_mut().drain(..));
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.worker.post_message(&"quit".into());
        self.worker.terminate();
    }
}

fn js_error(value: &wasm_bindgen::JsValue) -> String {
    value.as_string().unwrap_or_else(|| format!("{value:?}"))
}
