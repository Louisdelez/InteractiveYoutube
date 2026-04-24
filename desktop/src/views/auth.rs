//! Auth panel: drop-in replacement for the chat panel that lets the user
//! log in or register. Uses gpui-component Input widgets and submits via
//! reqwest in a background thread; the result is delivered back via mpsc.

use gpui::*;
use gpui_component::input::{Input, InputState};
use std::sync::mpsc::{self, Receiver, Sender};

use crate::i18n::t;
use crate::services::api::{self, User};

#[derive(Clone, Debug)]
pub enum AuthEvent {
    /// Successful login or register — close the panel and keep the user.
    Authenticated(User),
    /// User clicked the close button.
    Cancelled,
}

impl EventEmitter<AuthEvent> for AuthView {}

#[derive(Copy, Clone, PartialEq, Debug)]
enum Mode {
    Login,
    Register,
}

pub struct AuthView {
    mode: Mode,
    email: Entity<InputState>,
    password: Entity<InputState>,
    username: Entity<InputState>,
    error: Option<String>,
    pending: bool,
    result_tx: Sender<Result<User, String>>,
    result_rx: std::sync::Arc<std::sync::Mutex<Receiver<Result<User, String>>>>,
}

impl AuthView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let email = cx.new(|cx| InputState::new(window, cx).placeholder(t("auth.email.label")));
        let password = cx.new(|cx| InputState::new(window, cx).placeholder(t("auth.password.label")));
        let username = cx.new(|cx| InputState::new(window, cx).placeholder(t("auth.username.label")));

        let (tx, rx) = mpsc::channel::<Result<User, String>>();
        let rx = std::sync::Arc::new(std::sync::Mutex::new(rx));
        let rx_poll = rx.clone();

        // Poll the channel for results from the background thread.
        cx.spawn(async move |this, cx| {
            loop {
                let res = rx_poll.lock().ok().and_then(|r| r.try_recv().ok());
                if let Some(result) = res {
                    let _ = this.update(cx, |view: &mut AuthView, cx| {
                        view.pending = false;
                        match result {
                            Ok(user) => {
                                cx.emit(AuthEvent::Authenticated(user));
                            }
                            Err(msg) => {
                                view.error = Some(msg);
                                cx.notify();
                            }
                        }
                    });
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(80))
                    .await;
            }
        })
        .detach();

        Self {
            mode: Mode::Login,
            email,
            password,
            username,
            error: None,
            pending: false,
            result_tx: tx,
            result_rx: rx,
        }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            return;
        }
        let email = self.email.read(cx).value().to_string();
        let password = self.password.read(cx).value().to_string();
        let username = self.username.read(cx).value().to_string();
        let mode = self.mode;
        self.pending = true;
        self.error = None;
        let tx = self.result_tx.clone();
        std::thread::spawn(move || {
            let res = match mode {
                Mode::Login => api::login(&email, &password),
                Mode::Register => api::register(&username, &email, &password),
            };
            let _ = tx.send(res);
        });
    }
}

impl Render for AuthView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Keep the rx alive
        let _ = &self.result_rx;
        let mode = self.mode;
        let pending = self.pending;
        let error = self.error.clone();

        // Centered modal panel (the dark backdrop is rendered by AppView).
        div()
            .flex()
            .flex_col()
            .w(px(380.0))
            .bg(rgb(0x18181b))
            .border_1()
            .border_color(rgb(0x2d2d30))
            .rounded(px(8.0))
            .shadow_lg()
            .px_5()
            .py_5()
            .gap_3()
            // Header
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(0xefeff1))
                            .child(if mode == Mode::Login {
                                t("auth.login")
                            } else {
                                t("auth.register")
                            }),
                    )
                    .child(
                        div()
                            .id("auth-close")
                            .text_xs()
                            .text_color(rgb(0xaaaaaa))
                            .cursor_pointer()
                            .hover(|this| this.text_color(rgb(0xefeff1)))
                            .child("✕")
                            .on_click(cx.listener(|_this, _, _, cx| {
                                cx.emit(AuthEvent::Cancelled);
                            })),
                    ),
            )
            // Inputs
            .child({
                let mut wrap = div().flex().flex_col().gap_2();
                if mode == Mode::Register {
                    wrap = wrap.child(Input::new(&self.username));
                }
                wrap.child(Input::new(&self.email))
                    .child(Input::new(&self.password))
            })
            // Error
            .child(match error {
                Some(msg) => div().text_xs().text_color(rgb(0xef4444)).child(msg),
                None => div(),
            })
            // Submit
            .child(
                div()
                    .id("auth-submit")
                    .flex()
                    .items_center()
                    .justify_center()
                    .px_3()
                    .py_2()
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .bg(rgb(0x9b59b6))
                    .hover(|this| this.bg(rgb(0xb57edc)))
                    .text_xs()
                    .text_color(rgb(0xffffff))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(if pending {
                        t("auth.loading")
                    } else if mode == Mode::Login {
                        t("auth.login_confirm")
                    } else {
                        t("auth.register_confirm")
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.submit(cx);
                    })),
            )
            // Toggle mode
            .child(
                div()
                    .id("auth-toggle-mode")
                    .text_xs()
                    .text_color(rgb(0xaaaaaa))
                    .cursor_pointer()
                    .hover(|this| this.text_color(rgb(0x9b59b6)))
                    .child(if mode == Mode::Login {
                        t("auth.switch_to_register")
                    } else {
                        t("auth.switch_to_login")
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.mode = if this.mode == Mode::Login {
                            Mode::Register
                        } else {
                            Mode::Login
                        };
                        this.error = None;
                        cx.notify();
                    })),
            )
    }
}
