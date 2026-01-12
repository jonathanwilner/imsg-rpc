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
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};
use serde_json::Value;
use std::{
    collections::HashMap,
    io::{self, Stdout},
    sync::mpsc::Receiver,
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
    sender: String,
    text: String,
    created_at: String,
    is_from_me: bool,
}

#[derive(Debug)]
enum PendingRequest {
    Chats,
    History,
    WatchSubscribe,
    WatchUnsubscribe,
    Send,
}

#[derive(Debug)]
enum InputMode {
    Normal,
    SendText,
    SendTo,
    SendDirectText,
}

struct App {
    chats: Vec<Chat>,
    messages: Vec<Message>,
    selected: usize,
    status: String,
    watch_subscription: Option<String>,
    pending: HashMap<String, PendingRequest>,
    input: String,
    input_mode: InputMode,
    input_target: Option<String>,
    notify: bool,
    last_tick: Instant,
}

impl App {
    fn new(notify: bool) -> Self {
        Self {
            chats: Vec::new(),
            messages: Vec::new(),
            selected: 0,
            status: "ready".to_string(),
            watch_subscription: None,
            pending: HashMap::new(),
            input: String::new(),
            input_mode: InputMode::Normal,
            input_target: None,
            notify,
            last_tick: Instant::now(),
        }
    }
}

fn main() -> io::Result<()> {
    let args = Args::parse();

    let mut client = match args.transport {
        Transport::Local => RpcClient::connect_local(&args.imsg_bin, args.db.as_deref())?,
        Transport::Tcp => RpcClient::connect_tcp(&args.host, args.port)?,
    };

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, &mut client, args.notify);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    client: &mut RpcClient,
    notify: bool,
) -> io::Result<()> {
    let mut app = App::new(notify);
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

        handle_rpc_events(client.events(), &mut app);
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
    }
}

fn handle_normal_key(client: &mut RpcClient, app: &mut App, key: KeyEvent) -> io::Result<bool> {
    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Char('r') => request_chats(client, app),
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
        KeyCode::Enter => {
            if let Some(chat) = app.chats.get(app.selected) {
                request_history(client, app, chat.id);
            }
        }
        KeyCode::Char('w') => {
            if let Some(chat) = app.chats.get(app.selected) {
                toggle_watch(client, app, chat.id);
            }
        }
        KeyCode::Char('s') => {
            if app.chats.get(app.selected).is_some() {
                app.input_mode = InputMode::SendText;
                app.input.clear();
                app.status = "send: enter text (enter to send, esc to cancel)".to_string();
            } else {
                app.status = "no chat selected".to_string();
            }
        }
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

fn handle_input_to(_client: &mut RpcClient, app: &mut App, key: KeyEvent) -> io::Result<bool> {
    match key.code {
        KeyCode::Esc => {
            app.input_mode = InputMode::Normal;
            app.input.clear();
            app.status = "cancelled".to_string();
        }
        KeyCode::Enter => {
            if app.input.trim().is_empty() {
                app.status = "recipient required".to_string();
            } else {
                app.input_target = Some(app.input.trim().to_string());
                app.input.clear();
                app.input_mode = InputMode::SendDirectText;
                app.status = "send: enter text".to_string();
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

fn toggle_watch(client: &mut RpcClient, app: &mut App, chat_id: i64) {
    if app.watch_subscription.is_some() {
        if let Some(sub) = app.watch_subscription.clone() {
            let id = client.send_request(
                "watch.unsubscribe",
                Some(serde_json::json!({ "subscription": sub })),
            );
            app.pending.insert(id, PendingRequest::WatchUnsubscribe);
            app.status = "unsubscribing...".to_string();
        }
        return;
    }
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

fn handle_rpc_events(rx: &Receiver<RpcEvent>, app: &mut App) {
    while let Ok(event) = rx.try_recv() {
        match event {
            RpcEvent::Response { id, result } => {
                if let Some(pending) = app.pending.remove(&id) {
                    handle_response(app, pending, result);
                }
            }
            RpcEvent::Error { id, error } => {
                if let Some(request_id) = id {
                    app.pending.remove(&request_id);
                }
                app.status = format!("rpc error: {error}");
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
                        if app.notify && !message.is_from_me {
                            let _ = Notification::new()
                                .summary(&message.sender)
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
            }
        }
    }
}

fn handle_response(app: &mut App, pending: PendingRequest, result: Value) {
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
            app.status = "history loaded".to_string();
        }
        PendingRequest::WatchSubscribe => {
            if let Some(sub) = result.get("subscription") {
                app.watch_subscription = Some(sub.to_string().trim_matches('"').to_string());
                app.status = "watch subscribed".to_string();
            }
        }
        PendingRequest::WatchUnsubscribe => {
            app.watch_subscription = None;
            app.status = "watch unsubscribed".to_string();
        }
        PendingRequest::Send => {
            app.status = "sent".to_string();
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

fn parse_notification_message(params: &Value) -> Option<Message> {
    let message = params.get("message")?;
    parse_message(message)
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

    let chats_list = List::new(chats)
        .block(Block::default().title("Chats").borders(Borders::ALL))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("➤ ");
    frame.render_stateful_widget(chats_list, body[0], &mut app_state_list(app));

    let mut message_lines = Vec::new();
    for message in &app.messages {
        let direction = if message.is_from_me { "sent" } else { "recv" };
        let header = format!(
            "{} [{}] {}:",
            message.created_at, direction, message.sender
        );
        message_lines.push(Line::from(vec![Span::raw(header)]));
        message_lines.push(Line::from(vec![Span::raw(message.text.clone())]));
        message_lines.push(Line::from(vec![Span::raw("")]));
    }
    let messages = Paragraph::new(Text::from(message_lines))
        .block(Block::default().title("Messages").borders(Borders::ALL))
        .wrap(ratatui::widgets::Wrap { trim: true });
    frame.render_widget(messages, body[1]);

    let input_label = match app.input_mode {
        InputMode::Normal => "Status",
        InputMode::SendText => "Send message",
        InputMode::SendTo => "Send to",
        InputMode::SendDirectText => "Send message",
    };
    let status_text = if matches!(app.input_mode, InputMode::Normal) {
        app.status.clone()
    } else {
        app.input.clone()
    };
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
