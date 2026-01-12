use clap::{Parser, ValueEnum};
use iced::{
    alignment, executor, theme,
    widget::{
        button, column, container, row, scrollable, text, text_input, Column, Container,
    },
    Application, Command, Element, Length, Settings, Subscription, Theme,
};
use notify_rust::Notification;
use serde_json::Value;
use std::{collections::HashMap, time::{Duration, Instant}};

mod rpc;
use rpc::{RpcClient, RpcEvent};

#[derive(Debug, Parser)]
#[command(name = "imsg-gui", about = "COSMIC-style GUI for imsg RPC")]
struct Args {
    #[arg(long, value_enum, default_value = "local")]
    transport: Transport,
    #[arg(long, default_value = "imsg")]
    imsg_bin: String,
    #[arg(long)]
    db: Option<String>,
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 57999)]
    port: u16,
    #[arg(long, default_value_t = true)]
    notify: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Transport {
    Local,
    Tcp,
}

#[derive(Debug, Clone)]
struct Chat {
    id: i64,
    name: String,
    identifier: String,
    service: String,
    last_message_at: String,
}

#[derive(Debug, Clone)]
struct MessageRow {
    chat_id: i64,
    sender: String,
    text: String,
    created_at: String,
    is_from_me: bool,
}

#[derive(Debug, Clone)]
enum PendingRequest {
    Chats,
    History,
    WatchSubscribe,
    WatchUnsubscribe,
    Send,
}

#[derive(Debug, Clone)]
enum InputMode {
    None,
    SendChat,
    DirectTo,
    DirectText,
}

#[derive(Debug, Clone)]
enum AppMessage {
    Tick,
    RefreshChats,
    SelectChat(usize),
    LoadHistory,
    ToggleWatch,
    StartSend,
    StartDirect,
    InputChanged(String),
    SubmitInput,
    CancelInput,
}

struct App {
    client: Option<RpcClient>,
    pending: HashMap<String, PendingRequest>,
    chats: Vec<Chat>,
    messages: Vec<MessageRow>,
    selected: usize,
    watch_subscription: Option<String>,
    input_mode: InputMode,
    input_value: String,
    input_target: Option<String>,
    status: String,
    notify: bool,
    last_tick: Instant,
}

impl App {
    fn new(flags: Flags) -> (Self, Command<AppMessage>) {
        let mut status = "ready".to_string();
        let mut client = None;
        match flags.transport {
            Transport::Local => match RpcClient::connect_local(&flags.imsg_bin, flags.db.as_deref()) {
                Ok(c) => {
                    client = Some(c);
                }
                Err(err) => status = format!("rpc error: {err}"),
            },
            Transport::Tcp => match RpcClient::connect_tcp(&flags.host, flags.port) {
                Ok(c) => {
                    client = Some(c);
                }
                Err(err) => status = format!("rpc error: {err}"),
            },
        }

        let mut app = Self {
            client,
            pending: HashMap::new(),
            chats: Vec::new(),
            messages: Vec::new(),
            selected: 0,
            watch_subscription: None,
            input_mode: InputMode::None,
            input_value: String::new(),
            input_target: None,
            status,
            notify: flags.notify,
            last_tick: Instant::now(),
        };

        app.request_chats();
        (app, Command::none())
    }

    fn request_chats(&mut self) {
        if let Some(client) = &mut self.client {
            let id = client.send_request("chats.list", Some(serde_json::json!({ "limit": 50 })));
            self.pending.insert(id, PendingRequest::Chats);
            self.status = "loading chats...".to_string();
        }
    }

    fn request_history(&mut self, chat_id: i64) {
        if let Some(client) = &mut self.client {
            let id = client.send_request(
                "messages.history",
                Some(serde_json::json!({ "chat_id": chat_id, "limit": 50 })),
            );
            self.pending.insert(id, PendingRequest::History);
            self.status = format!("loading history for chat {}", chat_id);
        }
    }

    fn toggle_watch(&mut self, chat_id: i64) {
        if let Some(client) = &mut self.client {
            if let Some(sub) = self.watch_subscription.clone() {
                let id = client.send_request(
                    "watch.unsubscribe",
                    Some(serde_json::json!({ "subscription": sub })),
                );
                self.pending.insert(id, PendingRequest::WatchUnsubscribe);
                self.status = "unsubscribing...".to_string();
                return;
            }
            let id = client.send_request(
                "watch.subscribe",
                Some(serde_json::json!({ "chat_id": chat_id })),
            );
            self.pending.insert(id, PendingRequest::WatchSubscribe);
            self.status = "subscribing...".to_string();
        }
    }

    fn request_send_chat(&mut self, chat_id: i64, text: &str) {
        if let Some(client) = &mut self.client {
            let id = client.send_request(
                "send",
                Some(serde_json::json!({ "chat_id": chat_id, "text": text })),
            );
            self.pending.insert(id, PendingRequest::Send);
            self.status = "sending...".to_string();
        }
    }

    fn request_send_to(&mut self, to: &str, text: &str) {
        if let Some(client) = &mut self.client {
            let id = client.send_request(
                "send",
                Some(serde_json::json!({ "to": to, "text": text })),
            );
            self.pending.insert(id, PendingRequest::Send);
            self.status = "sending...".to_string();
        }
    }

    fn handle_rpc_event(&mut self, event: RpcEvent) {
        match event {
            RpcEvent::Response { id, result } => {
                if let Some(pending) = self.pending.remove(&id) {
                    self.handle_response(pending, result);
                }
            }
            RpcEvent::Error { id, error } => {
                if let Some(req_id) = id {
                    self.pending.remove(&req_id);
                }
                self.status = format!("rpc error: {error}");
            }
            RpcEvent::Notification { method, params } => {
                if method == "message" {
                    if let Some(message) = parse_notification_message(&params) {
                        let should_append = self
                            .chats
                            .get(self.selected)
                            .map(|chat| chat.id == message.chat_id)
                            .unwrap_or(false);
                        if should_append {
                            self.messages.push(message.clone());
                        }
                        if self.notify && !message.is_from_me {
                            let _ = Notification::new()
                                .summary(&message.sender)
                                .body(&message.text)
                                .appname("imsg")
                                .show();
                        }
                        self.status = "new message".to_string();
                    }
                }
            }
            RpcEvent::Closed { message } => {
                self.status = format!("rpc closed: {message}");
            }
        }
    }

    fn handle_response(&mut self, pending: PendingRequest, result: Value) {
        match pending {
            PendingRequest::Chats => {
                let chats = result
                    .get("chats")
                    .and_then(|v| v.as_array())
                    .map(|list| list.iter().filter_map(parse_chat).collect())
                    .unwrap_or_else(Vec::new);
                self.chats = chats;
                if self.selected >= self.chats.len() {
                    self.selected = 0;
                }
                self.status = "chats loaded".to_string();
            }
            PendingRequest::History => {
                let messages = result
                    .get("messages")
                    .and_then(|v| v.as_array())
                    .map(|list| list.iter().filter_map(parse_message).collect())
                    .unwrap_or_else(Vec::new);
                self.messages = messages;
                self.status = "history loaded".to_string();
            }
            PendingRequest::WatchSubscribe => {
                if let Some(sub) = result.get("subscription") {
                    self.watch_subscription = Some(sub.to_string().trim_matches('"').to_string());
                    self.status = "watch subscribed".to_string();
                }
            }
            PendingRequest::WatchUnsubscribe => {
                self.watch_subscription = None;
                self.status = "watch unsubscribed".to_string();
            }
            PendingRequest::Send => {
                self.status = "sent".to_string();
            }
        }
    }

    fn drain_events(&mut self) {
        let mut events = Vec::new();
        if let Some(client) = self.client.as_ref() {
            while let Ok(event) = client.events().try_recv() {
                events.push(event);
            }
        }
        for event in events {
            self.handle_rpc_event(event);
        }
    }
}

#[derive(Debug, Clone)]
struct Flags {
    transport: Transport,
    imsg_bin: String,
    db: Option<String>,
    host: String,
    port: u16,
    notify: bool,
}

impl Default for Flags {
    fn default() -> Self {
        Self {
            transport: Transport::Local,
            imsg_bin: "imsg".to_string(),
            db: None,
            host: "127.0.0.1".to_string(),
            port: 57999,
            notify: true,
        }
    }
}

impl Application for App {
    type Executor = executor::Default;
    type Message = AppMessage;
    type Theme = Theme;
    type Flags = Flags;

    fn new(flags: Self::Flags) -> (Self, Command<Self::Message>) {
        App::new(flags)
    }

    fn title(&self) -> String {
        "imsg - COSMIC".to_string()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        iced::time::every(Duration::from_millis(150)).map(|_| AppMessage::Tick)
    }

    fn update(&mut self, message: Self::Message) -> Command<Self::Message> {
        match message {
            AppMessage::Tick => {
                if self.last_tick.elapsed() >= Duration::from_millis(150) {
                    self.last_tick = Instant::now();
                    self.drain_events();
                }
            }
            AppMessage::RefreshChats => {
                self.request_chats();
            }
            AppMessage::SelectChat(index) => {
                self.selected = index;
            }
            AppMessage::LoadHistory => {
                if let Some(chat) = self.chats.get(self.selected) {
                    self.request_history(chat.id);
                }
            }
            AppMessage::ToggleWatch => {
                if let Some(chat) = self.chats.get(self.selected) {
                    self.toggle_watch(chat.id);
                }
            }
            AppMessage::StartSend => {
                self.input_mode = InputMode::SendChat;
                self.input_value.clear();
                self.status = "send: enter message".to_string();
            }
            AppMessage::StartDirect => {
                self.input_mode = InputMode::DirectTo;
                self.input_value.clear();
                self.input_target = None;
                self.status = "send: enter recipient".to_string();
            }
            AppMessage::InputChanged(value) => {
                self.input_value = value;
            }
            AppMessage::SubmitInput => match self.input_mode {
                InputMode::None => {}
                InputMode::SendChat => {
                    if let Some(chat) = self.chats.get(self.selected) {
                        let text = self.input_value.trim().to_string();
                        if !text.is_empty() {
                            self.request_send_chat(chat.id, &text);
                        }
                    }
                    self.input_mode = InputMode::None;
                    self.input_value.clear();
                }
                InputMode::DirectTo => {
                    let target = self.input_value.trim().to_string();
                    if !target.is_empty() {
                        self.input_target = Some(target);
                        self.input_value.clear();
                        self.input_mode = InputMode::DirectText;
                        self.status = "send: enter message".to_string();
                    }
                }
                InputMode::DirectText => {
                    let text = self.input_value.trim().to_string();
                    if let Some(target) = self.input_target.clone() {
                        if !text.is_empty() {
                            self.request_send_to(&target, &text);
                        }
                    }
                    self.input_mode = InputMode::None;
                    self.input_target = None;
                    self.input_value.clear();
                }
            },
            AppMessage::CancelInput => {
                self.input_mode = InputMode::None;
                self.input_value.clear();
                self.input_target = None;
                self.status = "cancelled".to_string();
            }
        }
        Command::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let cosmic_bg = iced::Color::from_rgb(0.08, 0.09, 0.11);
        let cosmic_panel = iced::Color::from_rgb(0.14, 0.15, 0.18);
        let cosmic_accent = iced::Color::from_rgb(0.29, 0.64, 0.96);
        let cosmic_text = iced::Color::from_rgb(0.92, 0.93, 0.94);

        let mut chat_items = Column::new().spacing(6);
        for (index, chat) in self.chats.iter().enumerate() {
            let label = if chat.name.is_empty() {
                format!("{} [{}] {}", chat.identifier, chat.service, chat.last_message_at)
            } else {
                format!(
                    "{} ({}) [{}] {}",
                    chat.name, chat.identifier, chat.service, chat.last_message_at
                )
            };

            let row = row![text(label).size(14)].padding(6);
            let is_selected = index == self.selected;
            let background = if is_selected { cosmic_accent } else { cosmic_panel };
            let button = button(Container::new(row).style(theme::Container::Custom(Box::new(
                CosmicContainerStyle {
                    background,
                    text_color: Some(cosmic_text),
                },
            ))))
                .on_press(AppMessage::SelectChat(index));
            chat_items = chat_items.push(button);
        }

        let chat_panel = Container::new(scrollable(chat_items).height(Length::Fill))
            .width(Length::FillPortion(3))
            .style(theme::Container::Custom(Box::new(CosmicContainerStyle {
                background: cosmic_panel,
                text_color: Some(cosmic_text),
            })));

        let mut message_items = Column::new().spacing(8);
        for message in &self.messages {
            let direction = if message.is_from_me { "sent" } else { "recv" };
            let header = format!("{} [{}] {}", message.created_at, direction, message.sender);
            message_items = message_items.push(text(header).size(14));
            message_items = message_items.push(text(message.text.clone()).size(16));
            message_items = message_items.push(text(" "));
        }

        let message_panel = Container::new(scrollable(message_items).height(Length::Fill))
            .width(Length::FillPortion(7))
            .style(theme::Container::Custom(Box::new(CosmicContainerStyle {
                background: cosmic_bg,
                text_color: Some(cosmic_text),
            })));

        let controls = row![
            button(text("Refresh")).on_press(AppMessage::RefreshChats),
            button(text("History")).on_press(AppMessage::LoadHistory),
            button(text("Watch")).on_press(AppMessage::ToggleWatch),
            button(text("Send")).on_press(AppMessage::StartSend),
            button(text("Direct")).on_press(AppMessage::StartDirect),
        ]
        .spacing(10);

        let input_placeholder = match self.input_mode {
            InputMode::None => "select Send or Direct",
            InputMode::SendChat => "message",
            InputMode::DirectTo => "recipient",
            InputMode::DirectText => "message",
        };

        let input = text_input(input_placeholder, &self.input_value)
            .on_input(AppMessage::InputChanged)
            .on_submit(AppMessage::SubmitInput)
            .padding(8)
            .style(theme::TextInput::Custom(Box::new(CosmicInputStyle)));

        let cancel = button(text("Cancel")).on_press(AppMessage::CancelInput);
        let status = text(&self.status)
            .size(14)
            .horizontal_alignment(alignment::Horizontal::Left);

        let footer = column![controls, row![input, cancel].spacing(10), status]
            .spacing(10)
            .padding(12);

        let content = column![
            row![chat_panel, message_panel].height(Length::Fill),
            footer
        ]
        .height(Length::Fill);

        Container::new(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .style(theme::Container::Custom(Box::new(CosmicContainerStyle {
                background: cosmic_bg,
                text_color: Some(cosmic_text),
            })))
            .into()
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}

struct CosmicInputStyle;

#[derive(Debug, Clone)]
struct CosmicContainerStyle {
    background: iced::Color,
    text_color: Option<iced::Color>,
}

impl container::StyleSheet for CosmicContainerStyle {
    type Style = Theme;

    fn appearance(&self, _style: &Self::Style) -> container::Appearance {
        container::Appearance {
            text_color: self.text_color,
            background: Some(self.background.into()),
            border: iced::Border::default(),
            shadow: iced::Shadow::default(),
        }
    }
}

impl text_input::StyleSheet for CosmicInputStyle {
    type Style = Theme;

    fn active(&self, _style: &Self::Style) -> text_input::Appearance {
        text_input::Appearance {
            background: iced::Color::from_rgb(0.14, 0.15, 0.18).into(),
            border: iced::Border {
                radius: 6.0.into(),
                width: 1.0,
                color: iced::Color::from_rgb(0.29, 0.64, 0.96),
            },
            icon_color: iced::Color::from_rgb(0.8, 0.85, 0.9),
        }
    }

    fn focused(&self, style: &Self::Style) -> text_input::Appearance {
        self.active(style)
    }

    fn placeholder_color(&self, _style: &Self::Style) -> iced::Color {
        iced::Color::from_rgb(0.55, 0.58, 0.62)
    }

    fn value_color(&self, _style: &Self::Style) -> iced::Color {
        iced::Color::from_rgb(0.92, 0.93, 0.94)
    }

    fn disabled_color(&self, _style: &Self::Style) -> iced::Color {
        iced::Color::from_rgb(0.45, 0.48, 0.52)
    }

    fn selection_color(&self, _style: &Self::Style) -> iced::Color {
        iced::Color::from_rgb(0.29, 0.64, 0.96)
    }

    fn disabled(&self, style: &Self::Style) -> text_input::Appearance {
        self.active(style)
    }
}

fn parse_chat(value: &Value) -> Option<Chat> {
    Some(Chat {
        id: value.get("id")?.as_i64()?,
        name: value.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        identifier: value
            .get("identifier")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        service: value
            .get("service")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        last_message_at: value
            .get("last_message_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

fn parse_message(value: &Value) -> Option<MessageRow> {
    Some(MessageRow {
        chat_id: value.get("chat_id")?.as_i64()?,
        sender: value.get("sender").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        text: value.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        created_at: value
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        is_from_me: value.get("is_from_me").and_then(|v| v.as_bool()).unwrap_or(false),
    })
}

fn parse_notification_message(params: &Value) -> Option<MessageRow> {
    let message = params.get("message")?;
    parse_message(message)
}

fn main() -> iced::Result {
    let args = Args::parse();
    let flags = Flags {
        transport: args.transport,
        imsg_bin: args.imsg_bin,
        db: args.db,
        host: args.host,
        port: args.port,
        notify: args.notify,
    };
    App::run(Settings {
        flags,
        antialiasing: true,
        ..Settings::default()
    })
}
