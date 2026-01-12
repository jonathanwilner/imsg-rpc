use clap::{Parser, ValueEnum};
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use notify_rust::Notification;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};
use linkify::LinkFinder;
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{self, Stdout},
    time::{Duration, Instant},
};

mod rpc;
use rpc::{RpcClient, RpcEvent};

#[derive(Debug, Parser)]
#[command(name = "imsg-tui", about = "Ratatui client for imsg RPC")]
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
struct RpcConfig {
    transport: Transport,
    imsg_bin: String,
    db: Option<String>,
    host: String,
    port: u16,
}

impl RpcConfig {
    fn connect(&self) -> io::Result<RpcClient> {
        match self.transport {
            Transport::Local => RpcClient::connect_local(&self.imsg_bin, self.db.as_deref()),
            Transport::Tcp => RpcClient::connect_tcp(&self.host, self.port),
        }
    }
}

#[derive(Debug, Clone)]
struct Chat {
    id: i64,
    name: String,
    identifier: String,
    last_message_at: String,
    service: String,
}

#[derive(Debug, Clone)]
struct Message {
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

#[derive(Debug)]
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

#[derive(Debug)]
enum InputMode {
    Normal,
    SendText,
    SendTo,
    SendDirectText,
    Reaction,
}

#[derive(Debug, Clone, Copy)]
enum FocusPane {
    Chats,
    Messages,
}

struct App {
    chats: Vec<Chat>,
    messages: Vec<Message>,
    selected: usize,
    status: String,
    watch_subscription: Option<String>,
    watch_chat_id: Option<i64>,
    pending: HashMap<String, PendingRequest>,
    contacts: HashMap<String, String>,
    input: String,
    input_mode: InputMode,
    input_target: Option<String>,
    reaction_target: Option<String>,
    contact_query: Option<String>,
    notify: bool,
    last_tick: Instant,
    focus: FocusPane,
    message_offset: usize,
    message_index: usize,
    reconnect_at: Option<Instant>,
    reconnect_attempts: u32,
    config: RpcConfig,
}

impl App {
    fn new(notify: bool, config: RpcConfig) -> Self {
        Self {
            chats: Vec::new(),
            messages: Vec::new(),
            selected: 0,
            status: "ready".to_string(),
            watch_subscription: None,
            watch_chat_id: None,
            pending: HashMap::new(),
            contacts: HashMap::new(),
            input: String::new(),
            input_mode: InputMode::Normal,
            input_target: None,
            reaction_target: None,
            contact_query: None,
            notify,
            last_tick: Instant::now(),
            focus: FocusPane::Chats,
            message_offset: 0,
            message_index: 0,
            reconnect_at: None,
            reconnect_attempts: 0,
            config,
        }
    }
}

fn main() -> io::Result<()> {
    let args = Args::parse();
    let config = RpcConfig {
        transport: args.transport,
        imsg_bin: args.imsg_bin,
        db: args.db,
        host: args.host,
        port: args.port,
    };

    let mut client = config.connect()?;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, &mut client, args.notify, config);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    client: &mut RpcClient,
    notify: bool,
    config: RpcConfig,
) -> io::Result<()> {
    let mut app = App::new(notify, config);
    request_chats(client, &mut app);

    loop {
        terminal.draw(|frame| ui(frame, &app))?;

        let timeout = Duration::from_millis(100);
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if handle_key(client, &mut app, key)? {
                    break;
                }
            }
        }

        handle_rpc_events(client, &mut app);
        handle_reconnect(client, &mut app);
        if app.last_tick.elapsed() > Duration::from_secs(5) {
            app.last_tick = Instant::now();
        }
    }

    Ok(())
}

fn handle_key(client: &mut RpcClient, app: &mut App, key: KeyEvent) -> io::Result<bool> {
    match app.input_mode {
        InputMode::Normal => handle_normal_key(client, app, key),
        InputMode::SendText => handle_input_text(client, app, key, false),
        InputMode::SendTo => handle_input_to(client, app, key),
        InputMode::SendDirectText => handle_input_text(client, app, key, true),
        InputMode::Reaction => handle_input_reaction(client, app, key),
    }
}

fn handle_normal_key(client: &mut RpcClient, app: &mut App, key: KeyEvent) -> io::Result<bool> {
    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Tab => {
            app.focus = match app.focus {
                FocusPane::Chats => FocusPane::Messages,
                FocusPane::Messages => FocusPane::Chats,
            };
        }
        KeyCode::Char('r') => request_chats(client, app),
        KeyCode::Up | KeyCode::Down => handle_arrow_navigation(app, key.code),
        KeyCode::Char('k') => handle_scroll_messages(app, -1),
        KeyCode::Char('j') => handle_scroll_messages(app, 1),
        KeyCode::PageUp => handle_scroll_messages(app, -10),
        KeyCode::PageDown => handle_scroll_messages(app, 10),
        KeyCode::Enter => handle_enter(client, app),
        KeyCode::Char('w') => handle_watch(client, app),
        KeyCode::Char('s') => handle_send(app),
        KeyCode::Char('o') => handle_open_url(app),
        KeyCode::Char('R') => handle_reaction(app),
        KeyCode::Char('n') => {
            app.input_mode = InputMode::SendTo;
            app.input.clear();
            app.input_target = None;
            app.status = "send: enter recipient".to_string();
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
        _ => {}
    }
    Ok(false)
}

fn handle_arrow_navigation(app: &mut App, code: KeyCode) {
    match app.focus {
        FocusPane::Chats => match code {
            KeyCode::Up => {
                if app.selected > 0 {
                    app.selected -= 1;
                }
            }
            KeyCode::Down => {
                if app.selected + 1 < app.chats.len() {
                    app.selected += 1;
                }
            }
            _ => {}
        },
        FocusPane::Messages => match code {
            KeyCode::Up => {
                if app.message_index > 0 {
                    app.message_index -= 1;
                    app.message_offset = app.message_index;
                }
            }
            KeyCode::Down => {
                if app.message_index + 1 < app.messages.len() {
                    app.message_index += 1;
                    app.message_offset = app.message_index;
                }
            }
            _ => {}
        },
    }
}

fn handle_scroll_messages(app: &mut App, delta: isize) {
    if app.messages.is_empty() {
        return;
    }
    let max_offset = app.messages.len().saturating_sub(1);
    let current = app.message_offset as isize;
    let next = (current + delta).clamp(0, max_offset as isize) as usize;
    app.message_offset = next;
}

fn handle_enter(client: &mut RpcClient, app: &mut App) {
    match app.focus {
        FocusPane::Chats => {
            if let Some(chat) = app.chats.get(app.selected) {
                request_history(client, app, chat.id);
                app.message_offset = 0;
                app.message_index = 0;
            }
        }
        FocusPane::Messages => {}
    }
}

fn handle_watch(client: &mut RpcClient, app: &mut App) {
    if let Some(chat) = app.chats.get(app.selected) {
        toggle_watch(client, app, chat.id);
    }
}

fn handle_send(app: &mut App) {
    if app.chats.get(app.selected).is_some() {
        app.input_mode = InputMode::SendText;
        app.input.clear();
        app.status = "send: enter text (enter to send, esc to cancel)".to_string();
    } else {
        app.status = "no chat selected".to_string();
    }
}

fn sender_display(app: &App, sender: &str) -> String {
    app.contacts
        .get(sender)
        .cloned()
        .unwrap_or_else(|| sender.to_string())
}

fn chat_service(app: &App, chat_id: i64) -> Option<&str> {
    app.chats
        .iter()
        .find(|chat| chat.id == chat_id)
        .map(|chat| chat.service.as_str())
}

fn bubble_style(message: &Message, service: Option<&str>) -> Style {
    if message.is_from_me {
        if matches!(service, Some("SMS") | Some("sms")) {
            Style::default().fg(Color::White).bg(Color::Green)
        } else {
            Style::default().fg(Color::White).bg(Color::Blue)
        }
    } else {
        Style::default().fg(Color::Black).bg(Color::Gray)
    }
}

fn current_message(app: &App) -> Option<&Message> {
    app.messages.get(app.message_index)
}

fn extract_urls(text: &str) -> Vec<String> {
    let mut finder = LinkFinder::new();
    finder.kinds(&[linkify::LinkKind::Url]);
    finder
        .links(text)
        .map(|link| link.as_str().to_string())
        .collect()
}

fn open_url(url: &str) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let mut cmd = std::process::Command::new("open");
    #[cfg(not(target_os = "macos"))]
    let mut cmd = std::process::Command::new("xdg-open");
    cmd.arg(url).spawn()?.wait()?;
    Ok(())
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

fn reply_preview(
    message: &Message,
    message_lookup: &HashMap<String, (String, String)>,
    contacts: &HashMap<String, String>,
) -> Option<String> {
    let reply_guid = message.reply_to_guid.as_ref()?;
    if let Some((sender, text)) = message_lookup.get(reply_guid) {
        let display = contacts.get(sender).cloned().unwrap_or_else(|| sender.clone());
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

fn styled_text_lines(text: &str, base_style: Style, link_style: Style) -> Vec<Line<'static>> {
    let mut finder = LinkFinder::new();
    finder.kinds(&[linkify::LinkKind::Url]);
    let mut lines = Vec::new();
    for raw_line in text.split('\n') {
        let mut spans: Vec<Span<'static>> = Vec::new();
        spans.push(Span::styled("  ", base_style));
        let mut last = 0;
        for link in finder.links(raw_line) {
            let start = link.start();
            let end = link.end();
            if start > last {
                spans.push(Span::styled(raw_line[last..start].to_string(), base_style));
            }
            spans.push(Span::styled(raw_line[start..end].to_string(), link_style));
            last = end;
        }
        if last < raw_line.len() {
            spans.push(Span::styled(raw_line[last..].to_string(), base_style));
        }
        spans.push(Span::styled("  ", base_style));
        lines.push(Line::from(spans));
    }
    if lines.is_empty() {
        lines.push(Line::from(vec![Span::styled("  ", base_style)]));
    }
    lines
}

fn handle_open_url(app: &mut App) {
    if let Some(message) = current_message(app) {
        let urls = extract_urls(&message.text);
        if let Some(url) = urls.first() {
            if open_url(url).is_ok() {
                app.status = format!("opened {url}");
            } else {
                app.status = "failed to open url".to_string();
            }
        } else {
            app.status = "no url found".to_string();
        }
    } else {
        app.status = "no message selected".to_string();
    }
}

fn handle_reaction(app: &mut App) {
    if let Some(guid) = current_message(app).map(|message| message.guid.clone()) {
        app.input_mode = InputMode::Reaction;
        app.input.clear();
        app.reaction_target = Some(guid);
        app.status = "react: enter reaction (like/love/laugh/...)".to_string();
    } else {
        app.status = "no message selected".to_string();
    }
}

fn handle_input_to(client: &mut RpcClient, app: &mut App, key: KeyEvent) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.input.clear();
            app.status = "cancelled".to_string();
        }
        KeyCode::Enter => {
            let value = app.input.trim().to_string();
            if value.is_empty() {
                app.status = "recipient required".to_string();
            } else {
                if looks_like_handle(&value) {
                    app.input_target = Some(value);
                    app.input.clear();
                    app.input_mode = InputMode::SendDirectText;
                    app.status = "send: enter text (enter to send, esc to cancel)".to_string();
                } else {
                    app.contact_query = Some(value);
                    app.status = "searching contacts...".to_string();
                    if let Some(query) = app.contact_query.clone() {
                        request_contact_search(client, app, &query);
                    }
                }
            }
        }
        KeyCode::Backspace => {
            app.input.pop();
        }
        KeyCode::Char(c) => app.input.push(c),
        _ => {}
    }
    Ok(false)
}

fn handle_input_text(
    client: &mut RpcClient,
    app: &mut App,
    key: KeyEvent,
    direct: bool,
) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.input.clear();
            app.status = "cancelled".to_string();
        }
        KeyCode::Enter => {
            let text = app.input.trim().to_string();
            if text.is_empty() {
                app.status = "message text required".to_string();
            } else {
                if direct {
                    if let Some(target) = app.input_target.clone() {
                        request_send_to(client, app, &target, &text);
                    }
                } else if let Some(chat) = app.chats.get(app.selected) {
                    request_send_chat(client, app, chat.id, &text);
                }
                app.input_mode = InputMode::Normal;
                app.input.clear();
                app.input_target = None;
            }
        }
        KeyCode::Backspace => {
            app.input.pop();
        }
        KeyCode::Char(c) => app.input.push(c),
        _ => {}
    }
    Ok(false)
}

fn handle_input_reaction(
    client: &mut RpcClient,
    app: &mut App,
    key: KeyEvent,
) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.input.clear();
            app.reaction_target = None;
            app.status = "cancelled".to_string();
        }
        KeyCode::Enter => {
            let reaction = app.input.trim().to_string();
            if reaction.is_empty() {
                app.status = "reaction required".to_string();
            } else if let Some(guid) = app.reaction_target.clone() {
                request_reaction(client, app, &guid, &reaction);
                app.input_mode = InputMode::Normal;
                app.input.clear();
                app.reaction_target = None;
            } else {
                app.status = "no message selected".to_string();
            }
        }
        KeyCode::Backspace => {
            app.input.pop();
        }
        KeyCode::Char(c) => app.input.push(c),
        _ => {}
    }
    Ok(false)
}

fn request_chats(client: &mut RpcClient, app: &mut App) {
    let id = client.send_request("chats.list", Some(serde_json::json!({ "limit": 50 })));
    app.pending.insert(id, PendingRequest::Chats);
    app.status = "loading chats...".to_string();
}

fn request_history(client: &mut RpcClient, app: &mut App, chat_id: i64) {
    let id = client.send_request(
        "messages.history",
        Some(serde_json::json!({ "chat_id": chat_id, "limit": 50 })),
    );
    app.pending.insert(id, PendingRequest::History);
    app.status = format!("loading history for chat {}", chat_id);
}

fn request_reaction(client: &mut RpcClient, app: &mut App, guid: &str, reaction: &str) {
    let id = client.send_request(
        "reactions.send",
        Some(serde_json::json!({ "guid": guid, "reaction": reaction })),
    );
    app.pending.insert(id, PendingRequest::Reaction);
    app.status = "sending reaction...".to_string();
}

fn request_contact_resolve(client: &mut RpcClient, app: &mut App, handles: &[String]) {
    let id = client.send_request(
        "contacts.resolve",
        Some(serde_json::json!({ "handles": handles })),
    );
    app.pending.insert(id, PendingRequest::ResolveContacts);
}

fn request_contact_search(client: &mut RpcClient, app: &mut App, query: &str) {
    let id = client.send_request(
        "contacts.search",
        Some(serde_json::json!({ "query": query, "limit": 10 })),
    );
    app.pending.insert(id, PendingRequest::ContactSearch);
}

fn toggle_watch(client: &mut RpcClient, app: &mut App, chat_id: i64) {
    if app.watch_subscription.is_some() {
        if let Some(sub) = app.watch_subscription.clone() {
            let id = client.send_request(
                "watch.unsubscribe",
                Some(serde_json::json!({ "subscription": sub })),
            );
            app.pending.insert(id, PendingRequest::WatchUnsubscribe);
            app.status = "unsubscribing...".to_string();
            app.watch_chat_id = None;
        }
        return;
    }
    app.watch_chat_id = Some(chat_id);
    let id = client.send_request(
        "watch.subscribe",
        Some(serde_json::json!({ "chat_id": chat_id })),
    );
    app.pending.insert(id, PendingRequest::WatchSubscribe);
    app.status = "subscribing...".to_string();
}

fn request_watch_subscribe(client: &mut RpcClient, app: &mut App, chat_id: i64) {
    let id = client.send_request(
        "watch.subscribe",
        Some(serde_json::json!({ "chat_id": chat_id })),
    );
    app.pending.insert(id, PendingRequest::WatchSubscribe);
    app.status = "subscribing...".to_string();
}

fn request_send_chat(client: &mut RpcClient, app: &mut App, chat_id: i64, text: &str) {
    let id = client.send_request(
        "send",
        Some(serde_json::json!({ "chat_id": chat_id, "text": text })),
    );
    app.pending.insert(id, PendingRequest::Send);
    app.status = "sending...".to_string();
}

fn request_send_to(client: &mut RpcClient, app: &mut App, to: &str, text: &str) {
    let id = client.send_request(
        "send",
        Some(serde_json::json!({ "to": to, "text": text })),
    );
    app.pending.insert(id, PendingRequest::Send);
    app.status = "sending...".to_string();
}

fn handle_rpc_events(client: &mut RpcClient, app: &mut App) {
    while let Ok(event) = client.events().try_recv() {
        match event {
            RpcEvent::Response { id, result } => {
                if let Some(pending) = app.pending.remove(&id) {
                    handle_response(client, app, pending, result);
                }
            }
            RpcEvent::Error { id, error } => {
                if let Some(request_id) = id {
                    if let Some(pending) = app.pending.remove(&request_id) {
                        handle_rpc_error(app, pending, &error);
                    } else {
                        app.status = format!("rpc error: {error}");
                    }
                } else {
                    app.status = format!("rpc error: {error}");
                }
            }
            RpcEvent::Notification { method, params } => {
                if method == "message" {
                    if let Some(message) = parse_notification_message(&params) {
                        let should_append = app
                            .chats
                            .get(app.selected)
                            .map(|chat| chat.id == message.chat_id)
                            .unwrap_or(false);
                        if should_append {
                            app.messages.push(message.clone());
                        }
                        if !app.contacts.contains_key(&message.sender) {
                            request_contact_resolve(client, app, &[message.sender.clone()]);
                        }
                        if app.notify && !message.is_from_me {
                            let sender = sender_display(app, &message.sender);
                            let _ = Notification::new()
                                .summary(&sender)
                                .body(&message.text)
                                .appname("imsg")
                                .show();
                        }
                        app.status = "new message".to_string();
                    }
                }
            }
            RpcEvent::Closed { message } => {
                app.status = format!("rpc closed: {message}");
                schedule_reconnect(app);
            }
        }
    }
}

fn handle_rpc_error(app: &mut App, pending: PendingRequest, error: &Value) {
    let code = error.get("code").and_then(|v| v.as_i64()).unwrap_or(0);
    let message = error
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("rpc error");
    match pending {
        PendingRequest::ResolveContacts => {
            app.status = "contacts unavailable; names disabled".to_string();
        }
        PendingRequest::ContactSearch => {
            app.status = "contact search unavailable; enter handle".to_string();
            if let Some(query) = app.contact_query.take() {
                app.input = query;
                app.input_mode = InputMode::SendTo;
            }
        }
        _ => {
            if code != 0 {
                app.status = format!("rpc error ({code}): {message}");
            } else {
                app.status = format!("rpc error: {message}");
            }
        }
    }
}

fn reconnect_delay(attempt: u32) -> Duration {
    let exp = attempt.min(4).saturating_sub(0);
    let seconds = 2_u64.saturating_mul(2_u64.saturating_pow(exp));
    Duration::from_secs(seconds.min(30))
}

fn schedule_reconnect(app: &mut App) {
    if app.reconnect_at.is_some() {
        return;
    }
    let delay = reconnect_delay(app.reconnect_attempts);
    app.reconnect_attempts = app.reconnect_attempts.saturating_add(1);
    app.reconnect_at = Some(Instant::now() + delay);
}

fn handle_reconnect(client: &mut RpcClient, app: &mut App) {
    let Some(when) = app.reconnect_at else { return };
    if Instant::now() < when {
        return;
    }
    match app.config.connect() {
        Ok(new_client) => {
            *client = new_client;
            app.reconnect_at = None;
            app.reconnect_attempts = 0;
            app.watch_subscription = None;
            app.pending.clear();
            app.status = "reconnected".to_string();
            request_chats(client, app);
            if let Some(chat_id) = app.watch_chat_id {
                request_watch_subscribe(client, app, chat_id);
            }
        }
        Err(err) => {
            app.status = format!("reconnect failed: {err}");
            app.reconnect_at = None;
            schedule_reconnect(app);
        }
    }
}

fn handle_response(client: &mut RpcClient, app: &mut App, pending: PendingRequest, result: Value) {
    match pending {
        PendingRequest::Chats => {
            let chats = result
                .get("chats")
                .and_then(|v| v.as_array())
                .map(|list| list.iter().filter_map(parse_chat).collect())
                .unwrap_or_else(Vec::new);
            app.chats = chats;
            if app.selected >= app.chats.len() {
                app.selected = 0;
            }
            app.status = "chats loaded".to_string();
        }
        PendingRequest::History => {
            let messages = result
                .get("messages")
                .and_then(|v| v.as_array())
                .map(|list| list.iter().filter_map(parse_message).collect())
                .unwrap_or_else(Vec::new);
            app.messages = messages;
            app.message_index = 0;
            app.message_offset = 0;
            app.status = "history loaded".to_string();
            let handles: Vec<String> = app
                .messages
                .iter()
                .map(|m| m.sender.clone())
                .filter(|h| !h.is_empty())
                .filter(|h| !app.contacts.contains_key(h))
                .collect();
            if !handles.is_empty() {
                request_contact_resolve(client, app, &handles);
            }
        }
        PendingRequest::WatchSubscribe => {
            if let Some(sub) = result.get("subscription") {
                app.watch_subscription = Some(sub.to_string().trim_matches('"').to_string());
                app.status = "watch subscribed".to_string();
            }
        }
        PendingRequest::WatchUnsubscribe => {
            app.watch_subscription = None;
            app.watch_chat_id = None;
            app.status = "watch unsubscribed".to_string();
        }
        PendingRequest::Send => {
            app.status = "sent".to_string();
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
                    app.contacts.insert(handle.to_string(), name.to_string());
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
            for entry in matches {
                if let Some(list) = entry.get("handles").and_then(|v| v.as_array()) {
                    for handle in list {
                        if let Some(value) = handle.as_str() {
                            handles.push(value.to_string());
                        }
                    }
                }
            }
            if handles.len() == 1 {
                app.input_target = Some(handles[0].clone());
                app.input_mode = InputMode::SendDirectText;
                app.status = "send: enter text (enter to send, esc to cancel)".to_string();
            } else if handles.is_empty() {
                app.status = "no contact matches; enter handle".to_string();
            } else {
                app.status = "multiple matches; enter handle".to_string();
            }
            app.contact_query = None;
        }
        PendingRequest::Reaction => {
            app.status = "reaction sent".to_string();
        }
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
        last_message_at: value
            .get("last_message_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        service: value
            .get("service")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

fn parse_message(value: &Value) -> Option<Message> {
    Some(Message {
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

fn parse_notification_message(params: &Value) -> Option<Message> {
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
    fn parse_message_includes_reactions_and_reply() {
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

fn ui(frame: &mut ratatui::Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)].as_ref())
        .split(frame.size());

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)].as_ref())
        .split(chunks[0]);

    let chats: Vec<ListItem> = app
        .chats
        .iter()
        .map(|chat| {
            let title = if chat.name.is_empty() {
                format!(
                    "{} [{}] last={}",
                    chat.identifier, chat.service, chat.last_message_at
                )
            } else {
                format!(
                    "{} ({}) [{}] last={}",
                    chat.name, chat.identifier, chat.service, chat.last_message_at
                )
            };
            ListItem::new(Line::from(vec![Span::raw(title)]))
        })
        .collect();

    let chats_title = match app.focus {
        FocusPane::Chats => "Chats *",
        FocusPane::Messages => "Chats",
    };
    let chats_list = List::new(chats)
        .block(Block::default().title(chats_title).borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("➤ ");
    frame.render_stateful_widget(chats_list, body[0], &mut app_state_list(app));

    let mut message_lines = Vec::new();
    let start = app.message_offset.min(app.messages.len());
    let mut message_lookup: HashMap<String, (String, String)> = HashMap::new();
    for message in &app.messages {
        if !message.guid.is_empty() {
            message_lookup.insert(message.guid.clone(), (message.sender.clone(), message.text.clone()));
        }
    }
    for (index, message) in app.messages.iter().enumerate().skip(start) {
        let service = chat_service(app, message.chat_id);
        let mut base_style = bubble_style(message, service);
        if matches!(app.focus, FocusPane::Messages) && index == app.message_index {
            base_style = base_style.add_modifier(Modifier::REVERSED);
        }
        let sender = sender_display(app, &message.sender);
        let header = format!("{} {}", message.created_at, sender);
        message_lines.push(Line::from(vec![Span::styled(
            format!("  {header}  "),
            base_style.add_modifier(Modifier::BOLD),
        )]));
        if let Some(reply_line) = reply_preview(message, &message_lookup, &app.contacts) {
            message_lines.push(Line::from(vec![Span::styled(
                format!("  {reply_line}  "),
                base_style.add_modifier(Modifier::ITALIC),
            )]));
        }
        let link_style = base_style.add_modifier(Modifier::UNDERLINED).fg(Color::LightBlue);
        let mut text_lines = styled_text_lines(&message.text, base_style, link_style);
        message_lines.append(&mut text_lines);
        if let Some(summary) = reaction_summary(&message.reactions) {
            message_lines.push(Line::from(vec![Span::styled(
                format!("  {summary}  "),
                base_style.add_modifier(Modifier::DIM),
            )]));
        }
        message_lines.push(Line::from(vec![Span::raw("")]));
    }
    let messages_title = match app.focus {
        FocusPane::Messages => "Messages *",
        FocusPane::Chats => "Messages",
    };
    let messages = Paragraph::new(Text::from(message_lines))
        .block(Block::default().title(messages_title).borders(Borders::ALL))
        .wrap(ratatui::widgets::Wrap { trim: true });
    frame.render_widget(messages, body[1]);

    let input_label = match app.input_mode {
        InputMode::Normal => "Status",
        InputMode::SendText => "Send message",
        InputMode::SendTo => "Send to",
        InputMode::SendDirectText => "Send message",
        InputMode::Reaction => "Reaction",
    };
    let mut status_text = if matches!(app.input_mode, InputMode::Normal) {
        app.status.clone()
    } else {
        app.input.clone()
    };
    if matches!(app.input_mode, InputMode::Normal) {
        status_text.push_str("\nkeys: Tab focus  Enter history  s send  n new  R react  o open");
    } else if matches!(app.input_mode, InputMode::SendTo) {
        status_text.push_str("\nenter a handle or name, Enter to confirm, Esc to cancel");
    }
    let status = Paragraph::new(status_text)
        .block(Block::default().title(input_label).borders(Borders::ALL));
    frame.render_widget(status, chunks[1]);
}

fn app_state_list(app: &App) -> ratatui::widgets::ListState {
    let mut state = ratatui::widgets::ListState::default();
    if !app.chats.is_empty() {
        state.select(Some(app.selected));
    }
    state
}
