use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event as TerminalEvent, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
    Frame, Terminal,
};
use soup::{config::Config, events::Event, world::World};

const GENOME_COLORS: [Color; 12] = [
    Color::Rgb(76, 201, 240),
    Color::Rgb(247, 37, 133),
    Color::Rgb(114, 239, 221),
    Color::Rgb(255, 190, 11),
    Color::Rgb(181, 23, 158),
    Color::Rgb(128, 255, 114),
    Color::Rgb(255, 89, 94),
    Color::Rgb(157, 78, 221),
    Color::Rgb(0, 245, 212),
    Color::Rgb(255, 146, 76),
    Color::Rgb(72, 149, 239),
    Color::Rgb(230, 255, 133),
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum DisplayMode {
    Genomes,
    Ancestors,
    Energy,
}

impl DisplayMode {
    fn next(self) -> Self {
        match self {
            Self::Genomes => Self::Ancestors,
            Self::Ancestors => Self::Energy,
            Self::Energy => Self::Genomes,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Genomes => "live genomes",
            Self::Ancestors => "ancestry",
            Self::Energy => "energy current",
        }
    }
}

#[derive(Clone, Copy)]
enum ActivityKind {
    Novel,
    Mutation,
    Birth,
    Death,
    Attack,
    Relationship,
}

struct Activity {
    tick: u64,
    text: String,
    kind: ActivityKind,
}

#[derive(Default)]
struct GenomeSummary {
    hash: u64,
    population: usize,
    max_generation: u32,
    max_drift: usize,
    total_energy: u64,
    harvested_a: u64,
    harvested_b: u64,
    given: u64,
    tag_seeks: u64,
    take_a_ops: u64,
    take_b_ops: u64,
}

struct App {
    world: World,
    config: Config,
    paused: bool,
    steps_per_frame: u64,
    selected_id: Option<u32>,
    display_mode: DisplayMode,
    known_genomes: HashSet<u64>,
    known_phenotypes: HashSet<(u64, &'static str)>,
    activity: VecDeque<Activity>,
    live_history: VecDeque<u64>,
    genome_history: VecDeque<u64>,
    mutation_sites: VecDeque<(u64, u16)>,
}

impl App {
    fn new(config: Config) -> Self {
        let world = World::new(config.clone());
        let selected_id = world.programs.keys().min().copied();
        let known_genomes = world
            .programs
            .values()
            .map(|program| world.genome_hash(program))
            .collect();
        let mut app = Self {
            world,
            config,
            paused: false,
            steps_per_frame: 100,
            selected_id,
            display_mode: DisplayMode::Genomes,
            known_genomes,
            known_phenotypes: HashSet::new(),
            activity: VecDeque::new(),
            live_history: VecDeque::new(),
            genome_history: VecDeque::new(),
            mutation_sites: VecDeque::new(),
        };
        app.sample();
        app
    }

    fn reset(&mut self) {
        let speed = self.steps_per_frame;
        *self = Self::new(self.config.clone());
        self.steps_per_frame = speed;
    }

    fn tick_once(&mut self) {
        let events = self.world.tick();
        self.observe(events);
    }

    fn advance(&mut self) {
        if self.paused {
            return;
        }
        for _ in 0..self.steps_per_frame {
            self.tick_once();
        }
        self.sample();
        self.repair_selection();
    }

    fn observe(&mut self, events: Vec<Event>) {
        for event in events {
            match event {
                Event::Mutated {
                    tick,
                    address,
                    old_value,
                    new_value,
                } => {
                    self.push_activity(
                        tick,
                        format!("mutation @{address:04x}  {old_value:02x} -> {new_value:02x}"),
                        ActivityKind::Mutation,
                    );
                    self.mutation_sites.push_front((tick, address));
                    self.mutation_sites.truncate(48);
                }
                Event::StructuralMutation {
                    tick,
                    id,
                    kind,
                    index,
                    old_length,
                    new_length,
                    ..
                } => {
                    self.push_activity(
                        tick,
                        format!(
                            "genome edit #{id}  {kind:?} @{index}  {old_length} -> {new_length} bytes"
                        ),
                        ActivityKind::Novel,
                    );
                    if let Some(program) = self.world.programs.get(&id) {
                        self.mutation_sites
                            .push_front((tick, program.start.wrapping_add(index)));
                        self.mutation_sites.truncate(48);
                    }
                }
                Event::TagChanged {
                    tick,
                    id,
                    old_tag,
                    new_tag,
                } => self.push_activity(
                    tick,
                    format!("recognition tag #{id}  {old_tag:02x} -> {new_tag:02x}"),
                    ActivityKind::Relationship,
                ),
                Event::ResourceTransfer {
                    tick,
                    donor_id,
                    receiver_id,
                    resource,
                    amount,
                } => self.push_activity(
                    tick,
                    format!("resource {resource:?}  #{donor_id} -> #{receiver_id}  {amount} units"),
                    ActivityKind::Relationship,
                ),
                Event::Metabolized {
                    tick,
                    id,
                    pathway,
                    input_a,
                    input_b,
                    energy_yield,
                } => self.push_activity(
                    tick,
                    format!(
                        "metabolism #{id} {pathway:?}  A:{input_a} B:{input_b} -> E:{energy_yield}"
                    ),
                    ActivityKind::Relationship,
                ),
                Event::Born {
                    tick,
                    id,
                    parent_id,
                    generation,
                    ..
                } => {
                    let Some(program) = self.world.programs.get(&id) else {
                        continue;
                    };
                    let hash = self.world.genome_hash(program);
                    let drift = self.world.ancestor_distance(program);
                    if self.known_genomes.insert(hash) {
                        self.push_activity(
                            tick,
                            format!(
                                "new genome {:06x}  gen {generation}  drift {drift}",
                                hash & 0xffffff
                            ),
                            ActivityKind::Novel,
                        );
                    } else if generation <= 2 || self.world.total_births.is_multiple_of(25) {
                        self.push_activity(
                            tick,
                            format!(
                                "birth #{id} <- #{}  gen {generation}",
                                parent_id.unwrap_or(0)
                            ),
                            ActivityKind::Birth,
                        );
                    }
                }
                Event::Died { tick, id, cause } => {
                    if self.world.total_deaths.is_multiple_of(20) {
                        self.push_activity(
                            tick,
                            format!("death #{id}  {cause:?}"),
                            ActivityKind::Death,
                        );
                    }
                }
                Event::ForeignWrite {
                    tick,
                    attacker_id,
                    victim_id,
                    address,
                } => {
                    self.mutation_sites.push_front((tick, address));
                    self.mutation_sites.truncate(48);
                    if self.world.total_foreign_writes.is_multiple_of(25) {
                        self.push_activity(
                            tick,
                            format!("attack #{attacker_id} -> #{victim_id}  @{address:04x}"),
                            ActivityKind::Attack,
                        );
                    }
                }
                _ => {}
            }
        }
    }

    fn push_activity(&mut self, tick: u64, text: String, kind: ActivityKind) {
        self.activity.push_front(Activity { tick, text, kind });
        self.activity.truncate(80);
    }

    fn sample(&mut self) {
        self.live_history.push_back(self.world.live_count() as u64);
        self.genome_history
            .push_back(self.world.live_genomes() as u64);
        while self.live_history.len() > 90 {
            self.live_history.pop_front();
            self.genome_history.pop_front();
        }
        let observed: Vec<_> = self
            .species()
            .into_iter()
            .map(|summary| (summary.hash, phenotype(&summary)))
            .filter(|(_, behavior)| *behavior != "unexpressed")
            .collect();
        for (hash, behavior) in observed {
            if self.known_phenotypes.insert((hash, behavior)) {
                self.push_activity(
                    self.world.tick,
                    format!("behavior {:06x}  {behavior}", hash & 0xffffff),
                    ActivityKind::Relationship,
                );
            }
        }
    }

    fn speed_up(&mut self) {
        self.steps_per_frame = (self.steps_per_frame * 10).min(100_000);
    }

    fn speed_down(&mut self) {
        self.steps_per_frame = (self.steps_per_frame / 10).max(1);
    }

    fn sorted_ids(&self) -> Vec<u32> {
        let mut ids: Vec<_> = self.world.programs.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    fn repair_selection(&mut self) {
        if self
            .selected_id
            .is_some_and(|id| self.world.programs.contains_key(&id))
        {
            return;
        }
        self.selected_id = self.sorted_ids().first().copied();
    }

    fn select(&mut self, direction: i32) {
        let ids = self.sorted_ids();
        if ids.is_empty() {
            self.selected_id = None;
            return;
        }
        let current = self
            .selected_id
            .and_then(|id| ids.iter().position(|candidate| *candidate == id))
            .unwrap_or(0);
        let next = if direction < 0 {
            current.saturating_sub(1)
        } else {
            (current + 1).min(ids.len() - 1)
        };
        self.selected_id = Some(ids[next]);
    }

    fn species(&self) -> Vec<GenomeSummary> {
        let mut by_hash: HashMap<u64, GenomeSummary> = HashMap::new();
        for program in self.world.programs.values() {
            let hash = self.world.genome_hash(program);
            let summary = by_hash.entry(hash).or_insert_with(|| GenomeSummary {
                hash,
                ..GenomeSummary::default()
            });
            summary.population += 1;
            summary.max_generation = summary.max_generation.max(program.generation);
            summary.max_drift = summary.max_drift.max(self.world.ancestor_distance(program));
            summary.total_energy += program.energy as u64;
            summary.harvested_a += program.trace.harvested_a;
            summary.harvested_b += program.trace.harvested_b;
            summary.given += program.trace.given_a + program.trace.given_b;
            summary.tag_seeks += program.trace.tag_seeks;
            summary.take_a_ops += program.trace.opcode_counts[31];
            summary.take_b_ops += program.trace.opcode_counts[37];
        }
        let mut species: Vec<_> = by_hash.into_values().collect();
        species.sort_by_key(|summary| {
            (
                std::cmp::Reverse(summary.population),
                std::cmp::Reverse(summary.max_generation),
            )
        });
        species
    }

    fn test_symbiosis(&mut self) {
        let horizon = 100_000;
        let Some(report) = self.world.counterfactual_symbiosis(horizon) else {
            self.push_activity(
                self.world.tick,
                "symbiosis test needs at least two live genomes".into(),
                ActivityKind::Relationship,
            );
            return;
        };
        self.push_activity(
            self.world.tick,
            format!(
                "counterfactual {:?}: {:06x} loses {:.0}%, {:06x} loses {:.0}% (births {} / {})",
                report.verdict,
                report.genome_a & 0xffffff,
                report.dependence_a * 100.0,
                report.genome_b & 0xffffff,
                report.dependence_b * 100.0,
                report.baseline_births_a,
                report.baseline_births_b,
            ),
            ActivityKind::Relationship,
        );
    }
}

fn genome_color(hash: u64) -> Color {
    GENOME_COLORS[(hash as usize) % GENOME_COLORS.len()]
}

fn ancestor_color(template_id: Option<u8>) -> Color {
    template_id
        .map(|id| GENOME_COLORS[id as usize % GENOME_COLORS.len()])
        .unwrap_or(Color::DarkGray)
}

fn resource_color(a: u32, b: u32) -> Color {
    match (a > 0, b > 0) {
        (false, false) => Color::Rgb(32, 36, 46),
        (true, false) => Color::Rgb(76, 201, 240),
        (false, true) => Color::Rgb(247, 37, 133),
        (true, true) => Color::Rgb(255, 209, 102),
    }
}

fn sparkline(values: &VecDeque<u64>, width: usize) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if values.is_empty() || width == 0 {
        return String::new();
    }
    let skip = values.len().saturating_sub(width);
    let visible: Vec<_> = values.iter().skip(skip).copied().collect();
    let min = visible.iter().min().copied().unwrap_or(0);
    let max = visible.iter().max().copied().unwrap_or(min);
    visible
        .into_iter()
        .map(|value| {
            let level = if max == min {
                3
            } else {
                ((value - min) * 7 / (max - min)) as usize
            };
            BARS[level]
        })
        .collect()
}

fn render_header(app: &App, frame: &mut Frame, area: Rect) {
    let world = &app.world;
    let max_drift = world
        .programs
        .values()
        .map(|program| world.ancestor_distance(program))
        .max()
        .unwrap_or(0);
    let running = if app.paused { "PAUSED" } else { "RUNNING" };
    let running_color = if app.paused {
        Color::Yellow
    } else {
        Color::Rgb(114, 239, 221)
    };
    let graph_width = area.width.saturating_sub(70) as usize;
    let lines = vec![
        Line::from(vec![
            Span::styled(
                " PRIMORDIAL SOUP ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(114, 239, 221))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {running}"),
                Style::default()
                    .fg(running_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "   tick {:>10}   counter-currents {} / {}   life {} ops   substitutions {:.2}%   structural {:.2}%",
                world.tick,
                world.config.energy_current,
                world.config.energy_decay_interval,
                world.config.max_program_age,
                world.config.mutation_rate * 100.0,
                (world.config.insertion_rate
                    + world.config.deletion_rate
                    + world.config.duplication_rate)
                    * 100.0
            )),
        ]),
        Line::from(vec![
            Span::styled(
                format!(" {:>4} ", world.live_count()),
                Style::default()
                    .fg(Color::Rgb(114, 239, 221))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("alive   "),
            Span::styled(
                format!("{:>4} ", world.live_genomes()),
                Style::default()
                    .fg(Color::Rgb(247, 37, 133))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("genomes   "),
            Span::styled(
                format!("G{:>3} ", world.max_generation),
                Style::default()
                    .fg(Color::Rgb(255, 190, 11))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "furthest generation   drift {:>3} bytes   births {}   deaths {}   mutations {}   attacks {}",
                max_drift,
                world.total_births,
                world.total_deaths,
                world.total_mutations,
                world.total_foreign_writes
            )),
        ]),
        Line::from(vec![
            Span::styled(" population ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                sparkline(&app.live_history, graph_width),
                Style::default().fg(Color::Rgb(114, 239, 221)),
            ),
            Span::styled("  diversity ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                sparkline(&app.genome_history, graph_width / 2),
                Style::default().fg(Color::Rgb(247, 37, 133)),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn range_has_recent_mutation(app: &App, start: usize, end: usize) -> bool {
    let lifetime = app.steps_per_frame.saturating_mul(40).max(2_000);
    app.mutation_sites.iter().any(|(tick, address)| {
        app.world.tick.saturating_sub(*tick) <= lifetime
            && (*address as usize) >= start
            && (*address as usize) < end
    })
}

fn render_memory(app: &App, frame: &mut Frame, area: Rect) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(" WORLD ", Style::default().add_modifier(Modifier::BOLD)),
            Span::styled(
                app.display_mode.label(),
                Style::default().fg(Color::DarkGray),
            ),
        ]))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(61, 68, 81)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let width = inner.width as usize;
    let height = inner.height as usize;
    let cells_per_pixel = 65536usize.div_ceil(width * height);
    let mut lines = Vec::with_capacity(height);

    for row in 0..height {
        let mut spans = Vec::with_capacity(width);
        for col in 0..width {
            let pixel = row * width + col;
            let start = (pixel * cells_per_pixel).min(65535);
            let end = ((pixel + 1) * cells_per_pixel).min(65536);
            let recent_mutation = range_has_recent_mutation(app, start, end);
            let (symbol, color) = match app.display_mode {
                DisplayMode::Energy => {
                    let resource_a = app.world.memory.energy_map[start..end]
                        .iter()
                        .copied()
                        .max()
                        .unwrap_or(0);
                    let resource_b = app.world.memory.resource_b_map[start..end]
                        .iter()
                        .copied()
                        .max()
                        .unwrap_or(0);
                    (
                        if resource_a == 0 && resource_b == 0 {
                            "·"
                        } else {
                            "█"
                        },
                        resource_color(resource_a, resource_b),
                    )
                }
                DisplayMode::Ancestors => {
                    let owner = (start..end).find_map(|address| app.world.addr_to_owner[address]);
                    let color = owner
                        .and_then(|id| app.world.programs.get(&id))
                        .map(|program| ancestor_color(program.template_id))
                        .unwrap_or(Color::Rgb(32, 36, 46));
                    (if owner.is_some() { "█" } else { "·" }, color)
                }
                DisplayMode::Genomes => {
                    let owner = (start..end).find_map(|address| app.world.addr_to_owner[address]);
                    let color = owner
                        .and_then(|id| app.world.programs.get(&id))
                        .map(|program| genome_color(app.world.genome_hash(program)))
                        .unwrap_or(Color::Rgb(32, 36, 46));
                    let color = if recent_mutation { Color::White } else { color };
                    (if owner.is_some() { "█" } else { "·" }, color)
                }
            };
            spans.push(Span::styled(symbol, Style::default().fg(color)));
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn short_origin(world: &World, template_id: Option<u8>) -> String {
    template_id
        .and_then(|id| world.template_names.get(id as usize))
        .map(|name| name.chars().take(9).collect())
        .unwrap_or_else(|| "unknown".to_string())
}

fn render_species(app: &App, frame: &mut Frame, area: Rect) {
    let species = app.species();
    let rows = species
        .iter()
        .take(area.height.saturating_sub(3) as usize)
        .map(|summary| {
            let color = genome_color(summary.hash);
            Row::new(vec![
                Cell::from("●").style(Style::default().fg(color)),
                Cell::from(format!("{:06x}", summary.hash & 0xffffff))
                    .style(Style::default().fg(color)),
                Cell::from(summary.population.to_string()),
                Cell::from(phenotype(summary)),
            ])
        });
    let widths = [
        Constraint::Length(2),
        Constraint::Length(7),
        Constraint::Length(4),
        Constraint::Min(10),
    ];
    let table = Table::new(rows, widths)
        .header(
            Row::new(["", "genome", "pop", "behavior"]).style(Style::default().fg(Color::DarkGray)),
        )
        .block(
            Block::default()
                .title(" DOMINANT GENOMES ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Rgb(61, 68, 81))),
        );
    frame.render_widget(table, area);
}

fn phenotype(summary: &GenomeSummary) -> &'static str {
    if summary.tag_seeks > 0 {
        "tag seeker"
    } else if summary.given > 0 {
        "resource donor"
    } else if summary.take_a_ops + summary.take_b_ops < 8 {
        "unexpressed"
    } else if summary.take_a_ops > summary.take_b_ops.saturating_mul(3) {
        "A specialist"
    } else if summary.take_b_ops > summary.take_a_ops.saturating_mul(3) {
        "B specialist"
    } else {
        "dual metabolism"
    }
}

fn activity_color(kind: ActivityKind) -> Color {
    match kind {
        ActivityKind::Novel => Color::Rgb(255, 190, 11),
        ActivityKind::Mutation => Color::Rgb(247, 37, 133),
        ActivityKind::Birth => Color::Rgb(114, 239, 221),
        ActivityKind::Death => Color::Rgb(135, 142, 153),
        ActivityKind::Attack => Color::Rgb(255, 89, 94),
        ActivityKind::Relationship => Color::Rgb(255, 209, 102),
    }
}

fn render_activity(app: &App, frame: &mut Frame, area: Rect) {
    let lines: Vec<_> = app
        .activity
        .iter()
        .take(area.height.saturating_sub(2) as usize)
        .map(|activity| {
            Line::from(vec![
                Span::styled(
                    format!(" {:>8} ", activity.tick),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(
                    &activity.text,
                    Style::default().fg(activity_color(activity.kind)),
                ),
            ])
        })
        .collect();
    let block = Block::default()
        .title(" EVOLUTION FEED ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Rgb(61, 68, 81)));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn mnemonic(byte: u8) -> &'static str {
    match byte {
        0 => "NOP",
        1 => "FWD",
        2 => "BACK",
        5 => "HOME",
        9 => "WRITE",
        10 => "COPY",
        12 => "IMM",
        20 => "JBACK",
        23 => "LOOP",
        24 => "END",
        25 => "ALLOC",
        26 => "BIRTH",
        27 => "SPLIT",
        30 => "EXCRETE-A",
        31 => "TAKE-A",
        32 => "SENSE-A",
        33 => "SIZE",
        34 => "SET-RH",
        35 => "SEEK-FOREIGN",
        36 => "EXCRETE-A@",
        37 => "TAKE-B",
        38 => "SENSE-B",
        39 => "EXCRETE-B",
        40 => "SEEK-A",
        41 => "SEEK-B",
        42 => "SET-TAG",
        43 => "SEEK-TAG",
        44 => "CONVERT-A",
        45 => "CONVERT-B",
        46 => "COMBINE-AB",
        255 => "HALT",
        47..=254 => "NOP*",
        _ => "OP",
    }
}

fn render_inspector(app: &App, frame: &mut Frame, area: Rect) {
    let Some(program) = app.selected_id.and_then(|id| app.world.programs.get(&id)) else {
        frame.render_widget(
            Paragraph::new(" no organisms alive")
                .block(Block::default().title(" ORGANISM ").borders(Borders::ALL)),
            area,
        );
        return;
    };
    let genome = app.world.memory.read_slice(program.start, program.length);
    let hash = app.world.genome_hash(program);
    let drift = app.world.ancestor_distance(program);
    let parent = program
        .parent_id
        .map(|id| id.to_string())
        .unwrap_or_else(|| "origin".into());
    let ip_offset = program.ip_offset() as usize;
    let mut bytes = Vec::new();
    let mut ops = Vec::new();
    for (index, byte) in genome.iter().take(24).enumerate() {
        let style = if index == ip_offset {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(255, 190, 11))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(genome_color(hash))
        };
        bytes.push(Span::styled(format!("{byte:02x} "), style));
        ops.push(Span::styled(format!("{} ", mnemonic(*byte)), style));
    }
    let lines = vec![
        Line::from(vec![
            Span::styled(
                format!(" #{} ", program.id),
                Style::default()
                    .fg(Color::Black)
                    .bg(genome_color(hash))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  genome {:06x}   generation {}   parent {}   ancestor {}   drift {} byte{}",
                hash & 0xffffff,
                program.generation,
                parent,
                short_origin(&app.world, program.template_id),
                drift,
                if drift == 1 { "" } else { "s" }
            )),
        ]),
        Line::from(format!(
            " @{:04x} len:{} ip:+{} age:{} E:{} stores A:{} B:{} tag:{:02x} regs A:{} B:{} RH:{:04x} WH:{:04x}",
            program.start,
            program.length,
            program.ip_offset(),
            program.age,
            program.energy,
            program.metabolite_a,
            program.metabolite_b,
            program.tag,
            program.reg_a,
            program.reg_b,
            program.rh,
            program.wh
        )),
        Line::from(bytes),
        Line::from(ops),
        Line::from(format!(
            " behavior  take A:{} B:{}  convert A:{} B:{} pairs:{}  excrete A:{} B:{}  tags:{}",
            program.trace.harvested_a,
            program.trace.harvested_b,
            program.trace.converted_a,
            program.trace.converted_b,
            program.trace.combined_ab,
            program.trace.given_a,
            program.trace.given_b,
            program.trace.tag_seeks,
        )),
    ];
    let block = Block::default()
        .title(" SELECTED ORGANISM ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(genome_color(hash)));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_footer(app: &App, frame: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled(
            " space ",
            Style::default().fg(Color::Black).bg(Color::White),
        ),
        Span::raw(" pause  "),
        Span::styled(" . ", Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw(" step  "),
        Span::styled(" +/- ", Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw(format!(" {}  ", app.steps_per_frame)),
        Span::styled(" v ", Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw(" view  "),
        Span::styled(" ↑↓ ", Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw(" inspect  "),
        Span::styled(" r ", Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw(" reset  "),
        Span::styled(" y ", Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw(" counterfactual  "),
        Span::styled(" q ", Style::default().fg(Color::Black).bg(Color::White)),
        Span::raw(" quit"),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render(app: &App, frame: &mut Frame) {
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(10),
        Constraint::Length(7),
        Constraint::Length(1),
    ])
    .split(frame.area());
    render_header(app, frame, chunks[0]);
    let main = Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(chunks[1]);
    render_memory(app, frame, main[0]);
    let side =
        Layout::vertical([Constraint::Percentage(55), Constraint::Percentage(45)]).split(main[1]);
    render_species(app, frame, side[0]);
    render_activity(app, frame, side[1]);
    render_inspector(app, frame, chunks[2]);
    render_footer(app, frame, chunks[3]);
}

fn run(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, mut app: App) -> io::Result<()> {
    let frame_interval = Duration::from_millis(50);
    let mut last_frame = Instant::now();
    loop {
        terminal.draw(|frame| render(&app, frame))?;
        let wait = frame_interval
            .checked_sub(last_frame.elapsed())
            .unwrap_or(Duration::ZERO);
        if event::poll(wait)? {
            if let TerminalEvent::Key(KeyEvent {
                code, modifiers, ..
            }) = event::read()?
            {
                match code {
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(())
                    }
                    KeyCode::Char('p') | KeyCode::Char(' ') => app.paused = !app.paused,
                    KeyCode::Char('.') | KeyCode::Char('s') => {
                        app.tick_once();
                        app.sample();
                        app.repair_selection();
                    }
                    KeyCode::Char('+') | KeyCode::Char('=') => app.speed_up(),
                    KeyCode::Char('-') => app.speed_down(),
                    KeyCode::Char('v') | KeyCode::Char('t') => {
                        app.display_mode = app.display_mode.next()
                    }
                    KeyCode::Char('r') => app.reset(),
                    KeyCode::Char('y') => app.test_symbiosis(),
                    KeyCode::Down => app.select(1),
                    KeyCode::Up => app.select(-1),
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
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

#[cfg(test)]
mod tests {
    use super::mnemonic;

    #[test]
    fn metabolic_opcodes_have_explicit_display_mnemonics() {
        let expected = [
            (30, "EXCRETE-A"),
            (31, "TAKE-A"),
            (32, "SENSE-A"),
            (33, "SIZE"),
            (34, "SET-RH"),
            (35, "SEEK-FOREIGN"),
            (36, "EXCRETE-A@"),
            (37, "TAKE-B"),
            (38, "SENSE-B"),
            (39, "EXCRETE-B"),
            (40, "SEEK-A"),
            (41, "SEEK-B"),
            (42, "SET-TAG"),
            (43, "SEEK-TAG"),
            (44, "CONVERT-A"),
            (45, "CONVERT-B"),
            (46, "COMBINE-AB"),
        ];

        for (byte, expected_mnemonic) in expected {
            assert_eq!(mnemonic(byte), expected_mnemonic, "opcode byte {byte}");
        }
    }
}
