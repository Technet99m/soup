use std::{
    io,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
    Frame, Terminal,
};
use soup::{config::Config, world::World};

/// Color palette: index 0 = free, 1–7 = program colors (cycle by ID).
const PALETTE: [Color; 8] = [
    Color::DarkGray,
    Color::Cyan,
    Color::Green,
    Color::Yellow,
    Color::Magenta,
    Color::Blue,
    Color::Red,
    Color::White,
];

fn program_color(id: u32) -> Color {
    PALETTE[(id as usize % 7) + 1]
}

struct App {
    world: World,
    paused: bool,
    steps_per_frame: u64,
    table_state: TableState,
    selected_id: Option<u32>,
    energy_overlay: bool,
}

impl App {
    fn new(config: Config) -> Self {
        let mut table_state = TableState::default();
        table_state.select(Some(0));
        App {
            world: World::new(config),
            paused: false,
            steps_per_frame: 100,
            table_state,
            selected_id: None,
            energy_overlay: false,
        }
    }

    fn advance(&mut self) {
        if !self.paused {
            for _ in 0..self.steps_per_frame {
                self.world.tick();
            }
        }
    }

    fn speed_up(&mut self) {
        self.steps_per_frame = (self.steps_per_frame * 10).min(1_000_000);
    }

    fn speed_down(&mut self) {
        self.steps_per_frame = (self.steps_per_frame / 10).max(1);
    }

    fn sorted_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.world.programs.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    fn select_next(&mut self) {
        let n = self.world.programs.len();
        if n == 0 {
            return;
        }
        let i = self.table_state.selected().unwrap_or(0);
        let next = (i + 1).min(n - 1);
        self.table_state.select(Some(next));
        self.selected_id = self.sorted_ids().get(next).copied();
    }

    fn select_prev(&mut self) {
        let n = self.world.programs.len();
        if n == 0 {
            return;
        }
        let i = self.table_state.selected().unwrap_or(0);
        let prev = i.saturating_sub(1);
        self.table_state.select(Some(prev));
        self.selected_id = self.sorted_ids().get(prev).copied();
    }
}

/// Build a 65536-element array mapping each byte address to a PALETTE index.
fn build_color_map(world: &World) -> Box<[u8; 65536]> {
    let mut map = Box::new([0u8; 65536]);
    // Sort by ID so higher IDs (newer programs) don't obscure lower ones visually
    let mut programs: Vec<_> = world.programs.values().collect();
    programs.sort_unstable_by_key(|p| p.id);
    for p in programs {
        let cidx = (p.id % 7 + 1) as u8;
        for i in 0..p.length {
            map[p.start.wrapping_add(i) as usize] = cidx;
        }
    }
    map
}

/// Map an energy deposit value to a yellow-scale color intensity.
/// Returns None for zero (no deposit), or Some(Color) for non-zero.
fn energy_color(deposit: u32) -> Option<Color> {
    if deposit == 0 {
        return None;
    }
    // log2 scale: 1–15 → dim yellow, 16–255 → yellow, 256+ → bright white-yellow
    let level = (deposit as f64).log2() as u32;
    let color = match level {
        0..=3  => Color::Rgb(80, 60, 0),
        4..=7  => Color::Rgb(160, 120, 0),
        8..=11 => Color::Rgb(220, 180, 0),
        _      => Color::Rgb(255, 240, 80),
    };
    Some(color)
}

fn render_memory(world: &World, energy_overlay: bool, frame: &mut Frame, area: Rect) {
    let total_deposited: u64 = world.memory.energy_map.iter().map(|&v| v as u64).sum();
    let overlay_label = if energy_overlay { "  [e:energy]" } else { "  [e:programs]" };
    let title = format!(
        " Memory  {} programs  {:.1}% used  deposited:{}{}",
        world.programs.len(),
        world.memory_utilization() * 100.0,
        total_deposited,
        overlay_label,
    );
    let block = Block::default().title(title).borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let w = inner.width as usize;
    let h = inner.height as usize;
    let total_cells = w * h;
    // how many bytes each terminal cell represents
    let bytes_per_cell = (65536 + total_cells - 1) / total_cells;

    let cmap = build_color_map(world);

    let mut lines: Vec<Line> = Vec::with_capacity(h);
    for row in 0..h {
        let mut spans: Vec<Span> = Vec::with_capacity(w);
        for col in 0..w {
            let cell_idx = row * w + col;
            let addr_start = (cell_idx * bytes_per_cell).min(65535);
            let addr_end = ((cell_idx + 1) * bytes_per_cell).min(65536);

            let (ch, color) = if energy_overlay {
                // Max energy in this byte range
                let max_e = (addr_start..addr_end)
                    .map(|a| world.memory.energy_map[a])
                    .max()
                    .unwrap_or(0);
                match energy_color(max_e) {
                    Some(c) => ("\u{2588}", c),
                    None    => ("\u{00B7}", Color::DarkGray),
                }
            } else {
                // Majority vote on non-free color in this byte range
                let mut counts = [0u16; 8];
                for addr in addr_start..addr_end {
                    counts[cmap[addr] as usize] += 1;
                }
                // Pick dominant occupied color (index 1–7), fall back to free (0)
                let dominant = (1u8..8).max_by_key(|&i| counts[i as usize]).unwrap_or(0);
                let occupied = counts[dominant as usize] > 0;
                if occupied && dominant > 0 {
                    ("\u{2588}", PALETTE[dominant as usize])
                } else {
                    ("\u{00B7}", Color::DarkGray)
                }
            };
            spans.push(Span::styled(ch, Style::default().fg(color)));
        }
        lines.push(Line::from(spans));
    }

    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

fn render_program_list(app: &mut App, frame: &mut Frame, area: Rect) {
    let initial_energy = app.world.config.initial_energy.max(1);
    let ids = app.sorted_ids();

    let rows: Vec<Row> = ids
        .iter()
        .map(|&id| {
            let p = &app.world.programs[&id];
            let pct = (p.energy as f64 / initial_energy as f64 * 100.0).min(100.0) as usize;
            let filled = pct / 10;
            let bar = format!(
                "{}{} {:>3}%",
                "\u{2588}".repeat(filled),
                "\u{2591}".repeat(10 - filled),
                pct
            );
            let color = program_color(id);
            Row::new(vec![
                Cell::from(id.to_string()).style(Style::default().fg(color)),
                Cell::from(p.start.to_string()),
                Cell::from(p.length.to_string()),
                Cell::from(p.age.to_string()),
                Cell::from(bar).style(Style::default().fg(color)),
            ])
        })
        .collect();

    let header = Row::new(["ID", "Start", "Len", "Age", "Energy"])
        .style(Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED));

    let widths = [
        Constraint::Length(5),
        Constraint::Length(6),
        Constraint::Length(4),
        Constraint::Length(8),
        Constraint::Min(14),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(
            Block::default()
                .title(format!(" Programs ({}) ", ids.len()))
                .borders(Borders::ALL),
        )
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    frame.render_stateful_widget(table, area, &mut app.table_state);
}

fn render_inspector(world: &World, selected_id: Option<u32>, frame: &mut Frame, area: Rect) {
    let text = match selected_id.and_then(|id| world.programs.get(&id)) {
        Some(p) => {
            let stack: Vec<String> = p.loop_stack.iter().map(|a| a.to_string()).collect();
            let parent = p
                .parent_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "\u{2014}".to_string());
            vec![
                Line::from(format!(
                    " ID:{id}  IP:{ip}  A:{a}  B:{b}  RH:{rh}  WH:{wh}  \
                     Energy:{e}  Age:{age}  Loop:[{stack}]",
                    id = p.id,
                    ip = p.ip,
                    a = p.reg_a,
                    b = p.reg_b,
                    rh = p.rh,
                    wh = p.wh,
                    e = p.energy,
                    age = p.age,
                    stack = stack.join(","),
                )),
                Line::from(format!(
                    " Parent:{parent}  Lineage:{}",
                    &p.lineage_id.to_string()[..8],
                )),
            ]
        }
        None => vec![Line::from(" \u{2191}\u{2193} select a program to inspect")],
    };

    let block = Block::default().title(" Inspector ").borders(Borders::ALL);
    let para = Paragraph::new(text).block(block);
    frame.render_widget(para, area);
}

fn render_statusbar(app: &App, frame: &mut Frame, area: Rect) {
    let status = if app.paused {
        Span::styled(
            "\u{23F8} PAUSED",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            "\u{25B6} RUNNING",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )
    };
    let info = Span::raw(format!(
        "  tick:{:>10}  speed:{:>7}x/frame  [p]ause  [s]tep  [+/-]speed  [\u{2191}\u{2193}]select  [e]nergy  [q]uit",
        app.world.tick, app.steps_per_frame,
    ));
    let line = Line::from(vec![status, info]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render(app: &mut App, frame: &mut Frame) {
    let area = frame.area();

    // Outer vertical split: status | main | inspector
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(8),
        Constraint::Length(4),
    ])
    .split(area);
    let status_area = chunks[0];
    let main_area = chunks[1];
    let inspector_area = chunks[2];

    // Main horizontal split: memory | program list
    let main_chunks = Layout::horizontal([
        Constraint::Percentage(62),
        Constraint::Percentage(38),
    ])
    .split(main_area);
    let memory_area = main_chunks[0];
    let list_area = main_chunks[1];

    render_statusbar(app, frame, status_area);
    render_memory(&app.world, app.energy_overlay, frame, memory_area);
    render_program_list(app, frame, list_area);
    render_inspector(&app.world, app.selected_id, frame, inspector_area);
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, mut app: App) -> io::Result<()> {
    let frame_interval = Duration::from_millis(50); // ~20 FPS
    let mut last_frame = Instant::now();

    loop {
        // Draw
        terminal.draw(|f| render(&mut app, f))?;

        // Poll for key events with a short timeout
        let wait = frame_interval
            .checked_sub(last_frame.elapsed())
            .unwrap_or(Duration::ZERO);

        if event::poll(wait)? {
            if let Event::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
            {
                match code {
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(())
                    }
                    KeyCode::Char('p') | KeyCode::Char(' ') => app.paused = !app.paused,
                    KeyCode::Char('s') => {
                        // Single step even when paused
                        app.world.tick();
                    }
                    KeyCode::Char('+') | KeyCode::Char('=') => app.speed_up(),
                    KeyCode::Char('-') => app.speed_down(),
                    KeyCode::Char('e') => app.energy_overlay = !app.energy_overlay,
                    KeyCode::Down => app.select_next(),
                    KeyCode::Up => app.select_prev(),
                    _ => {}
                }
            }
        }

        if last_frame.elapsed() >= frame_interval {
            app.advance();
            last_frame = Instant::now();
        }
    }
}

fn main() -> io::Result<()> {
    let config = Config::from_env();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run(&mut terminal, App::new(config));

    // Always restore terminal, even on error
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}
