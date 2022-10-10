use anyhow::bail;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{
    env, io,
    time::{Duration, Instant},
};
use synnax::cosmos::Cosmos;
use synnax::lcd::Lcd;
use synnax::query::contract::{Contract, ItemOrMap};
use tui::layout::Alignment;
use tui::widgets::Paragraph;
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Span, Spans},
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame, Terminal,
};

fn find_chain_by_prefix(contract_address: String) -> Result<String, anyhow::Error> {
    Ok(if contract_address.starts_with("ki") {
        String::from("https://api-mainnet.blockchain.ki")
    } else if contract_address.starts_with("tki") {
        String::from("https://api-challenge.blockchain.ki")
    } else if contract_address.starts_with("juno") {
        String::from("https://api-juno-ia.cosmosia.notional.ventures/")
    } else if contract_address.starts_with("osmo") {
        String::from("https://lcd.osmosis.zone/")
    } else if contract_address.starts_with("chihuahua") {
        String::from("https://api.chihuahua.wtf/")
    } else if contract_address.starts_with("stars") {
        String::from("https://rest.stargaze-apis.com/")
    } else {
        bail!("Invalid bech32 address => {}", contract_address);
    })
}

fn main() -> Result<(), anyhow::Error> {
    let args: Vec<String> = env::args().collect();

    env_logger::init();

    if args.len() != 2 {
        eprintln!("usage: ./cw-state contract_address");
        return Ok(());
    }

    let address = &args[1];

    let lcd = Lcd::new(if let Ok(lcd) = std::env::var("OVERLOAD_LCD") {
        lcd
    } else {
        find_chain_by_prefix(address.clone())?
    })?;
    let cosmos = Cosmos::new(&lcd);
    let contract = Contract::new(cosmos, address.clone())?;

    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // create app and run it
    let tick_rate = Duration::from_millis(250);
    let app = App::new(&contract);
    let res = run_app(&mut terminal, app, tick_rate);

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("{:?}", err)
    }

    Ok(())
}

#[derive(PartialEq)]
enum ListType {
    StateKeyList,
    MapKeyList,
}

struct StatefulList<'a> {
    state: ListState,
    items: Vec<String>,
    second_state: ListState,
    second_items: Vec<String>,
    contract: &'a Contract,
    current_list: ListType,
}

impl<'a> StatefulList<'a> {
    fn update_second_list(&mut self) {
        let value = self
            .contract
            .state
            .get(self.items[self.state.selected().unwrap()].as_str())
            .unwrap();

        if let ItemOrMap::Map { map } = value {
            self.second_items = Vec::from_iter(map.keys().cloned());
            self.second_state.select(Some(0usize));
        } else {
            self.second_state.select(None);
            self.second_items.clear();
        };
    }

    fn with_items(items: Vec<String>, contract: &'a Contract) -> StatefulList {
        let mut list = StatefulList {
            state: ListState::default(),
            items,
            second_state: ListState::default(),
            second_items: vec![],
            contract,
            current_list: ListType::StateKeyList,
        };
        list.state.select(Some(0usize));
        list.update_second_list();

        list
    }

    fn next(&mut self) {
        let (list_len, state, refresh_key) = match self.current_list {
            ListType::StateKeyList => (self.items.len(), &mut self.state, true),
            ListType::MapKeyList => (self.second_items.len(), &mut self.second_state, false),
        };

        if list_len == 0 {
            state.select(None);
            return;
        }

        let i = match state.selected() {
            Some(i) => {
                if i >= list_len - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        state.select(Some(i));

        if refresh_key {
            self.update_second_list();
        }
    }

    fn previous(&mut self) {
        let (list_len, state, refresh_key) = match self.current_list {
            ListType::StateKeyList => (self.items.len(), &mut self.state, true),
            ListType::MapKeyList => (self.second_items.len(), &mut self.second_state, false),
        };

        let i = match state.selected() {
            Some(i) => {
                if i == 0 {
                    list_len - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        state.select(Some(i));
        if refresh_key {
            self.update_second_list();
        }
    }

    fn go_right(&mut self) {
        if self.current_list == ListType::StateKeyList && !self.second_items.is_empty() {
            self.current_list = ListType::MapKeyList;
        }
    }

    fn go_left(&mut self) {
        if self.current_list == ListType::MapKeyList {
            self.current_list = ListType::StateKeyList;
        }
    }
}

struct App<'a> {
    items: StatefulList<'a>,
    events: Vec<(&'a str, &'a str)>,
    contract: &'a Contract,
}

impl<'a> App<'a> {
    fn new(contract: &'a Contract) -> App<'a> {
        let keys = contract.state.keys().cloned().collect();

        App {
            items: StatefulList::with_items(keys, contract),
            events: vec![("No", "Value")],
            contract,
        }
    }

    /// Rotate through the event list.
    /// This only exists to simulate some kind of "progress"
    fn on_tick(&mut self) {
        let event = self.events.remove(0);
        self.events.push(event);
    }
}

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: App,
    tick_rate: Duration,
) -> io::Result<()> {
    let mut last_tick = Instant::now();
    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));
        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Left => app.items.go_left(),
                    KeyCode::Right => app.items.go_right(),
                    KeyCode::Down => app.items.next(),
                    KeyCode::Up => app.items.previous(),
                    _ => {}
                }
            }
        }
        if last_tick.elapsed() >= tick_rate {
            app.on_tick();
            last_tick = Instant::now();
        }
    }
}

fn ui<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    // Create two chunks with equal horizontal screen space
    let global_panel = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
        .split(f.size());
    let chunks2 = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(80), Constraint::Percentage(20)].as_ref())
        .split(global_panel[1]);

    // Iterate through all elements in the `items` app and append some debug text to it.
    let items: Vec<ListItem> = app
        .items
        .items
        .iter()
        .map(|i| {
            let lines = vec![Spans::from(i.as_str())];
            ListItem::new(lines).style(Style::default().fg(Color::Gray))
        })
        .collect();

    // Create a List from all list items and highlight the currently selected one
    let items = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("State key"))
        .highlight_style(
            Style::default()
                .bg(match app.items.current_list {
                    ListType::StateKeyList => Color::LightYellow,
                    _ => Color::Yellow,
                })
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");

    // We can now render the item list
    f.render_stateful_widget(items, global_panel[0], &mut app.items.state);

    // Let's do the same for the events.
    // The event list doesn't have any state and only displays the current state of the list.
    let second_key: Vec<ListItem> = app
        .items
        .second_items
        .iter()
        .map(|i| {
            let lines = vec![Spans::from(i.as_str())];
            ListItem::new(lines).style(Style::default().fg(Color::Gray))
        })
        .collect();
    let second_key = List::new(second_key)
        .block(Block::default().borders(Borders::ALL).title("Map Key"))
        .highlight_style(
            Style::default()
                .bg(match app.items.current_list {
                    ListType::MapKeyList => Color::LightYellow,
                    _ => Color::Yellow,
                })
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");
    f.render_stateful_widget(second_key, chunks2[0], &mut app.items.second_state);

    let block = Block::default().borders(Borders::ALL).title("State Value");
    let paragraph = Paragraph::new(Spans::from(Span::styled(
        match app.items.state.selected() {
            None => "NO KEY SELECTED",
            Some(idx) => {
                let value = app
                    .contract
                    .state
                    .get(app.items.items[idx].as_str())
                    .unwrap();

                match value {
                    ItemOrMap::Item { value } => value.as_str(),
                    ItemOrMap::Map { map } => map
                        .get(
                            app.items.second_items[app.items.second_state.selected().unwrap()]
                                .as_str(),
                        )
                        .unwrap(),
                }
            }
        },
        Style::default().add_modifier(Modifier::ITALIC),
    )))
    .style(Style::default().bg(Color::Black).fg(Color::White))
    .block(block)
    .alignment(Alignment::Center);

    f.render_widget(paragraph, chunks2[1]);
}
