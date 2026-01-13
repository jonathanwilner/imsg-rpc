use clap::{Parser, ValueEnum};
use iced::{
    alignment, executor, theme,
    widget::{
        button, column, container, horizontal_space, pick_list, row, scrollable, text,
        text_editor, text_input, Column, Container,
    },
    Application, Command, Element, Length, Settings, Subscription, Theme,
};
use linkify::LinkFinder;
use notify_rust::Notification;
use serde_json::Value;
use std::{
    collections::HashMap,
    process::Command as ProcessCommand,
    time::{Duration, Instant},
};

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
    participants: Vec<String>,
}

#[derive(Debug, Clone)]
struct MessageRow {
    chat_id: i64,
    guid: String,
    reply_to_guid: Option<String>,
    sender: String,
    text: String,
    created_at: String,
    is_from_me: bool,
    reactions: Vec<Reaction>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct Reaction {
    emoji: String,
    sender: String,
    is_from_me: bool,
}

#[derive(Debug, Clone)]
enum PendingRequest {
    Chats,
    History,
    WatchSubscribe,
    WatchUnsubscribe,
    Send,
    ResolveContacts,
    ContactSearch,
    Reaction,
}

#[derive(Debug, Clone)]
enum InputMode {
    None,
    Reaction,
}

#[derive(Debug, Clone)]
enum AppMessage {
    Tick,
    RefreshChats,
    SelectChat(usize),
    SelectMessage(usize),
    LoadHistory,
    ToggleWatch,
    StartReaction,
    ComposeToChanged(String),
    ComposeAction(text_editor::Action),
    SendCompose,
    ClearCompose,
    ToggleHelp,
    ReactionInputChanged(String),
    SubmitReaction,
    CancelReaction,
    OpenUrl(String),
}

struct App {
    client: Option<RpcClient>,
    pending: HashMap<String, PendingRequest>,
    chats: Vec<Chat>,
    messages: Vec<MessageRow>,
    selected: usize,
    selected_message: Option<usize>,
    watch_subscription: Option<String>,
    watch_chat_id: Option<i64>,
    input_mode: InputMode,
    reaction_input: String,
    reaction_target: Option<String>,
    compose_to: String,
    compose_content: text_editor::Content,
    status: String,
    notify: bool,
    last_tick: Instant,
    contacts: HashMap<String, String>,
    contact_query: Option<String>,
    reconnect_at: Option<Instant>,
    reconnect_attempts: u32,
    config: Flags,
    recipient_history: Vec<String>,
    show_help: bool,
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
            selected_message: None,
            watch_subscription: None,
            watch_chat_id: None,
            input_mode: InputMode::None,
            reaction_input: String::new(),
            reaction_target: None,
            compose_to: String::new(),
            compose_content: text_editor::Content::new(),
            status,
            notify: flags.notify,
            last_tick: Instant::now(),
            contacts: HashMap::new(),
            contact_query: None,
            reconnect_at: None,
            reconnect_attempts: 0,
            config: flags,
            recipient_history: Vec::new(),
            show_help: false,
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
                self.watch_chat_id = None;
                return;
            }
            self.watch_chat_id = Some(chat_id);
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

    fn request_watch_subscribe(&mut self, chat_id: i64) {
        if let Some(client) = &mut self.client {
            let id = client.send_request(
                "watch.subscribe",
                Some(serde_json::json!({ "chat_id": chat_id })),
            );
            self.pending.insert(id, PendingRequest::WatchSubscribe);
            self.status = "subscribing...".to_string();
        }
    }

    fn request_reaction(&mut self, guid: &str, reaction: &str) {
        if let Some(client) = &mut self.client {
            let id = client.send_request(
                "reactions.send",
                Some(serde_json::json!({ "guid": guid, "reaction": reaction })),
            );
            self.pending.insert(id, PendingRequest::Reaction);
            self.status = "sending reaction...".to_string();
        }
    }

    fn request_contact_search(&mut self, query: &str) {
        if let Some(client) = &mut self.client {
            let id = client.send_request(
                "contacts.search",
                Some(serde_json::json!({ "query": query, "limit": 10 })),
            );
            self.pending.insert(id, PendingRequest::ContactSearch);
        }
    }

    fn request_contact_resolve(&mut self, handles: &[String]) {
        if let Some(client) = &mut self.client {
            let id = client.send_request(
                "contacts.resolve",
                Some(serde_json::json!({ "handles": handles })),
            );
            self.pending.insert(id, PendingRequest::ResolveContacts);
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
                        if !self.contacts.contains_key(&message.sender) {
                            self.request_contact_resolve(&[message.sender.clone()]);
                        }
                        if self.notify && !message.is_from_me {
                            let sender = self
                                .contacts
                                .get(&message.sender)
                                .cloned()
                                .unwrap_or(message.sender.clone());
                            let _ = Notification::new()
                                .summary(&sender)
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
                self.schedule_reconnect();
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
                let handles: Vec<String> = self
                    .chats
                    .iter()
                    .flat_map(|chat| {
                        let mut entries = Vec::new();
                        if !chat.identifier.is_empty() {
                            entries.push(chat.identifier.clone());
                        }
                        entries.extend(chat.participants.iter().cloned());
                        entries
                    })
                    .filter(|handle| !handle.is_empty())
                    .filter(|handle| !self.contacts.contains_key(handle))
                    .collect();
                if !handles.is_empty() {
                    self.request_contact_resolve(&handles);
                }
            }
            PendingRequest::History => {
                let messages = result
                    .get("messages")
                    .and_then(|v| v.as_array())
                    .map(|list| list.iter().filter_map(parse_message).collect())
                    .unwrap_or_else(Vec::new);
                self.messages = messages;
                self.selected_message = None;
                self.status = "history loaded".to_string();
                let handles: Vec<String> = self
                    .messages
                    .iter()
                    .map(|m| m.sender.clone())
                    .filter(|h| !h.is_empty())
                    .filter(|h| !self.contacts.contains_key(h))
                    .collect();
                if !handles.is_empty() {
                    self.request_contact_resolve(&handles);
                }
            }
            PendingRequest::WatchSubscribe => {
                if let Some(sub) = result.get("subscription") {
                    self.watch_subscription = Some(sub.to_string().trim_matches('"').to_string());
                    self.status = "watch subscribed".to_string();
                }
            }
            PendingRequest::WatchUnsubscribe => {
                self.watch_subscription = None;
                self.watch_chat_id = None;
                self.status = "watch unsubscribed".to_string();
            }
            PendingRequest::Send => {
                self.status = "sent".to_string();
            }
            PendingRequest::ResolveContacts => {
                let contacts = result
                    .get("contacts")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                for entry in contacts {
                    if let (Some(handle), Some(name)) = (
                        entry.get("handle").and_then(|v| v.as_str()),
                        entry.get("name").and_then(|v| v.as_str()),
                    ) {
                        self.contacts.insert(handle.to_string(), name.to_string());
                    }
                }
            }
        PendingRequest::ContactSearch => {
            let matches = result
                .get("matches")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let mut handles = Vec::new();
            let mut labels = Vec::new();
            for entry in matches {
                let name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(list) = entry.get("handles").and_then(|v| v.as_array()) {
                    for handle in list {
                        if let Some(value) = handle.as_str() {
                            handles.push(value.to_string());
                            if !name.is_empty() {
                                labels.push(format!("{name} <{value}>"));
                            } else {
                                labels.push(value.to_string());
                            }
                        }
                    }
                }
            }
            if handles.len() == 1 {
                self.compose_to = handles[0].clone();
                let body = self.compose_content.text().trim().to_string();
                if !body.is_empty() && self.contact_query.is_some() {
                    let target = self.compose_to.clone();
                    self.request_send_to(&target, &body);
                    self.record_recipient(&target);
                    self.compose_content = text_editor::Content::new();
                    self.status = "sent".to_string();
                } else {
                    self.status = "contact resolved".to_string();
                }
            } else if handles.is_empty() {
                self.status = "no contact matches; enter handle".to_string();
            } else {
                self.status = format!("multiple matches: {}", labels.join(", "));
            }
            self.contact_query = None;
        }
            PendingRequest::Reaction => {
                self.status = "reaction sent".to_string();
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

    fn record_recipient(&mut self, handle: &str) {
        let trimmed = handle.trim();
        if trimmed.is_empty() {
            return;
        }
        self.recipient_history.retain(|item| item != trimmed);
        self.recipient_history.insert(0, trimmed.to_string());
    }

    fn schedule_reconnect(&mut self) {
        if self.reconnect_at.is_some() {
            return;
        }
        let delay = reconnect_delay(self.reconnect_attempts);
        self.reconnect_attempts = self.reconnect_attempts.saturating_add(1);
        self.reconnect_at = Some(Instant::now() + delay);
    }

    fn handle_reconnect(&mut self) {
        let Some(when) = self.reconnect_at else { return };
        if Instant::now() < when {
            return;
        }
        match connect_from_config(&self.config) {
            Ok(client) => {
                self.client = Some(client);
                self.reconnect_at = None;
                self.reconnect_attempts = 0;
                self.watch_subscription = None;
                self.pending.clear();
                self.status = "reconnected".to_string();
                self.request_chats();
                if let Some(chat_id) = self.watch_chat_id {
                    self.request_watch_subscribe(chat_id);
                }
            }
            Err(err) => {
                self.status = format!("reconnect failed: {err}");
                self.reconnect_at = None;
                self.schedule_reconnect();
            }
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
                    self.handle_reconnect();
                }
            }
            AppMessage::RefreshChats => {
                self.request_chats();
            }
            AppMessage::SelectChat(index) => {
                self.selected = index;
                self.selected_message = None;
            }
            AppMessage::SelectMessage(index) => {
                self.selected_message = Some(index);
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
            AppMessage::StartReaction => {
                if let Some(index) = self.selected_message {
                    if let Some(message) = self.messages.get(index) {
                        if message.guid.is_empty() {
                            self.status = "selected message missing guid".to_string();
                        } else {
                            self.input_mode = InputMode::Reaction;
                            self.reaction_input.clear();
                            self.reaction_target = Some(message.guid.clone());
                            self.status = "reaction: enter reaction".to_string();
                        }
                    }
                } else {
                    self.status = "select a message first".to_string();
                }
            }
            AppMessage::ComposeToChanged(value) => {
                self.compose_to = value;
            }
            AppMessage::ComposeAction(action) => {
                self.compose_content.perform(action);
            }
            AppMessage::SendCompose => {
                let text = self.compose_content.text().trim().to_string();
                if text.is_empty() {
                    self.status = "message text required".to_string();
                    return Command::none();
                }
                let target = self.compose_to.trim().to_string();
                if target.is_empty() {
                    if let Some(chat) = self.chats.get(self.selected).cloned() {
                        self.request_send_chat(chat.id, &text);
                        if !chat.identifier.is_empty() {
                            self.record_recipient(&chat.identifier);
                        }
                        self.compose_content = text_editor::Content::new();
                        self.status = "sent".to_string();
                    } else {
                        self.status = "no chat selected".to_string();
                    }
                } else if looks_like_handle(&target) {
                    self.request_send_to(&target, &text);
                    self.record_recipient(&target);
                    self.compose_content = text_editor::Content::new();
                    self.status = "sent".to_string();
                } else {
                    self.contact_query = Some(target.clone());
                    self.status = "searching contacts...".to_string();
                    self.request_contact_search(&target);
                }
            }
            AppMessage::ClearCompose => {
                self.compose_to.clear();
                self.compose_content = text_editor::Content::new();
                self.status = "cleared".to_string();
            }
            AppMessage::ToggleHelp => {
                self.show_help = !self.show_help;
            }
            AppMessage::ReactionInputChanged(value) => {
                self.reaction_input = value;
            }
            AppMessage::SubmitReaction => {
                let reaction = self.reaction_input.trim().to_string();
                if let Some(target) = self.reaction_target.clone() {
                    if !reaction.is_empty() {
                        self.request_reaction(&target, &reaction);
                        self.status = "reaction sent".to_string();
                    } else {
                        self.status = "reaction required".to_string();
                    }
                }
                self.input_mode = InputMode::None;
                self.reaction_input.clear();
                self.reaction_target = None;
            }
            AppMessage::CancelReaction => {
                self.input_mode = InputMode::None;
                self.reaction_input.clear();
                self.reaction_target = None;
                self.status = "reaction cancelled".to_string();
            }
            AppMessage::OpenUrl(url) => {
                if open_url(&url).is_ok() {
                    self.status = format!("opened {url}");
                } else {
                    self.status = "failed to open url".to_string();
                }
            }
        }
        Command::none()
    }

    fn view(&self) -> Element<'_, Self::Message> {
        let cosmic_bg = iced::Color::from_rgb(0.08, 0.09, 0.11);
        let cosmic_panel = iced::Color::from_rgb(0.14, 0.15, 0.18);
        let cosmic_accent = iced::Color::from_rgb(0.29, 0.64, 0.96);
        let cosmic_text = iced::Color::from_rgb(0.92, 0.93, 0.94);
        let imessage_blue = iced::Color::from_rgb8(0, 122, 255);
        let sms_green = iced::Color::from_rgb8(52, 199, 89);
        let bubble_gray = iced::Color::from_rgb8(229, 229, 234);
        let bubble_text_dark = iced::Color::from_rgb8(24, 24, 24);

        let mut chat_items = Column::new().spacing(6);
        for (index, chat) in self.chats.iter().enumerate() {
            let contact_name = self
                .contacts
                .get(&chat.identifier)
                .cloned()
                .unwrap_or_default();
            let mut participants: Vec<String> = chat
                .participants
                .iter()
                .filter(|value| !value.is_empty())
                .map(|handle| self.contacts.get(handle).cloned().unwrap_or_else(|| handle.clone()))
                .collect();
            participants.retain(|value| !value.is_empty());
            participants.sort();
            participants.dedup();
            let participant_label = if participants.is_empty() {
                String::new()
            } else {
                let display = participants.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
                if participants.len() > 3 {
                    format!("{display} +{}", participants.len().saturating_sub(3))
                } else {
                    display
                }
            };
            let label = if chat.name.is_empty() {
                if !contact_name.is_empty() {
                    format!(
                        "{} ({}) [{}] {}",
                        contact_name, chat.identifier, chat.service, chat.last_message_at
                    )
                } else if !participant_label.is_empty() {
                    format!("{} [{}] {}", participant_label, chat.service, chat.last_message_at)
                } else if chat.identifier.is_empty() {
                    format!("[{}] {}", chat.service, chat.last_message_at)
                } else {
                    format!("{} [{}] {}", chat.identifier, chat.service, chat.last_message_at)
                }
            } else if chat.identifier.is_empty() {
                format!("{} [{}] {}", chat.name, chat.service, chat.last_message_at)
            } else if contact_name.is_empty() || contact_name == chat.name {
                format!(
                    "{} ({}) [{}] {}",
                    chat.name, chat.identifier, chat.service, chat.last_message_at
                )
            } else {
                format!(
                    "{} ({}, {}) [{}] {}",
                    chat.name, contact_name, chat.identifier, chat.service, chat.last_message_at
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

        let mut message_lookup: HashMap<String, (String, String)> = HashMap::new();
        for message in &self.messages {
            if !message.guid.is_empty() {
                message_lookup.insert(message.guid.clone(), (message.sender.clone(), message.text.clone()));
            }
        }

        let mut message_items = Column::new().spacing(10);
        for (index, message) in self.messages.iter().enumerate() {
            let sender = sender_display(&self.contacts, &message.sender);
            let header = format!("{} {}", message.created_at, sender);
            let service = self
                .chats
                .iter()
                .find(|chat| chat.id == message.chat_id)
                .map(|chat| chat.service.as_str());
            let background = if message.is_from_me {
                if matches!(service, Some("SMS") | Some("sms")) {
                    sms_green
                } else {
                    imessage_blue
                }
            } else {
                bubble_gray
            };
            let text_color = if message.is_from_me {
                iced::Color::WHITE
            } else {
                bubble_text_dark
            };
            let mut bubble_contents = Column::new().spacing(4);
            bubble_contents = bubble_contents.push(text(header).size(12).style(text_color));
            if let Some(reply) = reply_preview(message, &message_lookup, &self.contacts) {
                bubble_contents = bubble_contents.push(
                    text(reply)
                        .size(12)
                        .style(iced::Color::from_rgb8(90, 90, 90)),
                );
            }
            bubble_contents = bubble_contents.push(text(message.text.clone()).size(16).style(text_color));
            let urls = extract_urls(&message.text);
            if !urls.is_empty() {
                let mut link_row = row![];
                for url in urls {
                    let link = button(text(&url).size(12).style(imessage_blue))
                        .on_press(AppMessage::OpenUrl(url.clone()))
                        .style(theme::Button::Text);
                    link_row = link_row.push(link);
                }
                bubble_contents = bubble_contents.push(link_row.spacing(6));
            }
            if let Some(summary) = reaction_summary(&message.reactions) {
                bubble_contents = bubble_contents.push(
                    text(summary)
                        .size(12)
                        .style(iced::Color::from_rgb8(90, 90, 90)),
                );
            }
            let is_selected = self.selected_message == Some(index);
            let bubble = Container::new(bubble_contents)
                .padding(8)
                .style(theme::Container::Custom(Box::new(BubbleStyle {
                    background,
                    text_color: Some(text_color),
                    border_color: if is_selected { Some(cosmic_accent) } else { None },
                })));
            let bubble_button = button(bubble)
                .on_press(AppMessage::SelectMessage(index))
                .style(theme::Button::Text);
            let aligned = if message.is_from_me {
                row![horizontal_space(), bubble_button]
            } else {
                row![bubble_button, horizontal_space()]
            };
            message_items = message_items.push(aligned);
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
            button(text("React")).on_press(AppMessage::StartReaction),
            button(text("Help")).on_press(AppMessage::ToggleHelp),
        ]
        .spacing(10);

        let to_input = text_input("to (handle or name)", &self.compose_to)
            .on_input(AppMessage::ComposeToChanged)
            .padding(8)
            .style(theme::TextInput::Custom(Box::new(CosmicInputStyle)));
        let recent_pick = pick_list(
            self.recipient_history.clone(),
            None::<String>,
            AppMessage::ComposeToChanged,
        )
        .placeholder("recent");
        let editor = text_editor(&self.compose_content)
            .on_action(AppMessage::ComposeAction)
            .height(Length::Fixed(100.0));
        let send = button(text("Send")).on_press(AppMessage::SendCompose);
        let clear = button(text("Clear")).on_press(AppMessage::ClearCompose);

        let compose_row = row![to_input, recent_pick, send, clear].spacing(10);
        let status = text(&self.status)
            .size(14)
            .horizontal_alignment(alignment::Horizontal::Left);

        let footer = column![controls, compose_row, editor, status]
            .spacing(10)
            .padding(12);

        if self.show_help {
            return Container::new(text(help_text()).size(16))
                .padding(24)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(theme::Container::Custom(Box::new(CosmicContainerStyle {
                    background: iced::Color::from_rgb(0.08, 0.09, 0.11),
                    text_color: Some(cosmic_text),
                })))
                .into();
        }

        if matches!(self.input_mode, InputMode::Reaction) {
            let overlay = column![
                text("Send reaction").size(18),
                text_input("reaction (like/love/emoji)", &self.reaction_input)
                    .on_input(AppMessage::ReactionInputChanged)
                    .on_submit(AppMessage::SubmitReaction)
                    .padding(8)
                    .style(theme::TextInput::Custom(Box::new(CosmicInputStyle))),
                row![
                    button(text("Send")).on_press(AppMessage::SubmitReaction),
                    button(text("Cancel")).on_press(AppMessage::CancelReaction),
                ]
                .spacing(10),
            ]
            .spacing(10);
            return Container::new(overlay)
                .padding(24)
                .width(Length::Fill)
                .height(Length::Fill)
                .style(theme::Container::Custom(Box::new(CosmicContainerStyle {
                    background: iced::Color::from_rgb(0.08, 0.09, 0.11),
                    text_color: Some(cosmic_text),
                })))
                .into();
        }

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

fn connect_from_config(config: &Flags) -> std::io::Result<RpcClient> {
    match config.transport {
        Transport::Local => RpcClient::connect_local(&config.imsg_bin, config.db.as_deref()),
        Transport::Tcp => RpcClient::connect_tcp(&config.host, config.port),
    }
}

fn reconnect_delay(attempt: u32) -> Duration {
    let seconds = 2_u64.saturating_mul(2_u64.saturating_pow(attempt.min(4)));
    Duration::from_secs(seconds.min(30))
}

fn open_url(url: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = ProcessCommand::new("open");
    #[cfg(not(target_os = "macos"))]
    let mut cmd = ProcessCommand::new("xdg-open");
    cmd.arg(url).spawn()?;
    Ok(())
}

fn help_text() -> &'static str {
    "imsg-gui help\n\
Refresh: reload chats\n\
History: load messages for selected chat\n\
Watch: toggle streaming for selected chat\n\
React: send a reaction to the selected message\n\
Help: toggle this overlay\n\
\n\
Compose\n\
To: leave empty to send to selected chat\n\
Use Recent to pick previous recipients\n\
Send: send compose message\n\
Clear: reset compose fields\n"
}

fn looks_like_handle(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains('@') {
        return true;
    }
    trimmed.chars().all(|c| c.is_ascii_digit() || "+()- ".contains(c))
}

fn sender_display(contacts: &HashMap<String, String>, sender: &str) -> String {
    contacts
        .get(sender)
        .cloned()
        .unwrap_or_else(|| sender.to_string())
}

fn extract_urls(text: &str) -> Vec<String> {
    let mut finder = LinkFinder::new();
    finder.kinds(&[linkify::LinkKind::Url]);
    finder
        .links(text)
        .map(|link| link.as_str().to_string())
        .collect()
}

fn reply_preview(
    message: &MessageRow,
    message_lookup: &HashMap<String, (String, String)>,
    contacts: &HashMap<String, String>,
) -> Option<String> {
    let reply_guid = message.reply_to_guid.as_ref()?;
    if let Some((sender, text)) = message_lookup.get(reply_guid) {
        let display = sender_display(contacts, sender);
        let mut snippet = text.clone();
        if snippet.len() > 48 {
            snippet.truncate(48);
            snippet.push_str("…");
        }
        Some(format!("↪ {display}: {snippet}"))
    } else {
        Some(format!("↪ reply to {reply_guid}"))
    }
}

fn reaction_summary(reactions: &[Reaction]) -> Option<String> {
    if reactions.is_empty() {
        return None;
    }
    let mut counts: HashMap<String, usize> = HashMap::new();
    for reaction in reactions {
        *counts.entry(reaction.emoji.clone()).or_insert(0) += 1;
    }
    let mut parts: Vec<String> = counts
        .into_iter()
        .map(|(emoji, count)| {
            if count > 1 {
                format!("{emoji} {count}")
            } else {
                emoji
            }
        })
        .collect();
    parts.sort();
    Some(parts.join(" "))
}

struct CosmicInputStyle;

#[derive(Debug, Clone)]
struct CosmicContainerStyle {
    background: iced::Color,
    text_color: Option<iced::Color>,
}

#[derive(Debug, Clone)]
struct BubbleStyle {
    background: iced::Color,
    text_color: Option<iced::Color>,
    border_color: Option<iced::Color>,
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

impl container::StyleSheet for BubbleStyle {
    type Style = Theme;

    fn appearance(&self, _style: &Self::Style) -> container::Appearance {
        container::Appearance {
            text_color: self.text_color,
            background: Some(self.background.into()),
            border: iced::Border {
                radius: 12.0.into(),
                width: if self.border_color.is_some() { 1.0 } else { 0.0 },
                color: self.border_color.unwrap_or(self.background),
            },
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
        participants: value
            .get("participants")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|entry| entry.as_str().map(|value| value.to_string()))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn parse_message(value: &Value) -> Option<MessageRow> {
    Some(MessageRow {
        chat_id: value.get("chat_id")?.as_i64()?,
        guid: value.get("guid").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        reply_to_guid: value
            .get("reply_to_guid")
            .and_then(|v| v.as_str())
            .map(|value| value.to_string()),
        sender: value.get("sender").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        text: value.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        created_at: value
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        is_from_me: value.get("is_from_me").and_then(|v| v.as_bool()).unwrap_or(false),
        reactions: value
            .get("reactions")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|entry| {
                        let emoji = entry
                            .get("emoji")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if emoji.is_empty() {
                            return None;
                        }
                        Some(Reaction {
                            emoji,
                            sender: entry
                                .get("sender")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string(),
                            is_from_me: entry
                                .get("is_from_me")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn parse_notification_message(params: &Value) -> Option<MessageRow> {
    let message = params.get("message")?;
    parse_message(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_chat_handles_minimal_fields() {
        let value = serde_json::json!({
            "id": 1,
            "identifier": "+123",
            "service": "iMessage",
            "last_message_at": "2026-01-01T00:00:00Z",
            "name": ""
        });
        let chat = parse_chat(&value).expect("chat");
        assert_eq!(chat.id, 1);
        assert_eq!(chat.identifier, "+123");
        assert_eq!(chat.service, "iMessage");
        assert!(chat.participants.is_empty());
    }

    #[test]
    fn parse_message_handles_minimal_fields() {
        let value = serde_json::json!({
            "chat_id": 2,
            "sender": "+123",
            "text": "hello",
            "created_at": "2026-01-01T00:00:00Z",
            "is_from_me": false
        });
        let message = parse_message(&value).expect("message");
        assert_eq!(message.chat_id, 2);
        assert_eq!(message.sender, "+123");
        assert_eq!(message.text, "hello");
    }

    #[test]
    fn parse_message_includes_reply_and_reactions() {
        let value = serde_json::json!({
            "chat_id": 2,
            "guid": "ABC",
            "reply_to_guid": "DEF",
            "sender": "+123",
            "text": "hello",
            "created_at": "2026-01-01T00:00:00Z",
            "is_from_me": false,
            "reactions": [
                { "emoji": "❤️", "sender": "+123", "is_from_me": false }
            ]
        });
        let message = parse_message(&value).expect("message");
        assert_eq!(message.guid, "ABC");
        assert_eq!(message.reply_to_guid.as_deref(), Some("DEF"));
        assert_eq!(message.reactions.len(), 1);
        assert_eq!(message.reactions[0].emoji, "❤️");
    }

    #[test]
    fn reconnect_delay_caps_at_thirty_seconds() {
        assert_eq!(reconnect_delay(0).as_secs(), 2);
        assert_eq!(reconnect_delay(1).as_secs(), 4);
        assert_eq!(reconnect_delay(2).as_secs(), 8);
        assert_eq!(reconnect_delay(4).as_secs(), 30);
        assert_eq!(reconnect_delay(10).as_secs(), 30);
    }
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
