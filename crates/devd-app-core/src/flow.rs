//! The shape of a team run: who works on what, and what waits for what.
//!
//! A run's tasks form a graph: each depends on others, and nothing starts before what it depends on is
//! done. Drawn as a chain it answers the questions a list cannot: what is running now, what is it holding up,
//! and where did it stop. This is only the arithmetic of the picture, with no window in it, so it can be tested:
//! which column each task is in, which row, and the path each dependency takes between them.
//!
//! The layout is the standard layered one. A task sits one column to the right of the last task it depends
//! on, so every line runs left to right and none doubles back. A dependency that skips columns is carried
//! through the columns it skips on a lane of its own, so no line passes behind a task. The order of tasks
//! within a column is chosen to keep lines from crossing.

use std::collections::HashMap;

/// One task, as the run reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// Its key in the plan, which what depends on it names.
    pub key: String,
    /// The role that owns it.
    pub role: String,
    /// What it is for.
    pub title: String,
    /// Where it stands: `pending`, `running`, `blocked`, `done`…
    pub state: String,
    /// The conversation it is working in, once it has started.
    pub thread: Option<String>,
    /// The keys of what it waits for.
    pub depends_on: Vec<String>,
    /// Seconds since it last did anything (a step, a tool call, a reply), when it has begun.
    pub quiet_secs: Option<u64>,
    /// How many times it has been started.
    pub attempts: u32,
}

/// A task that has been placed.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Its place in the list it was built from.
    pub task: usize,
    /// Its column: how many tasks lie between it and the start.
    pub column: usize,
    /// Its centre, in the picture's own units.
    pub center: (f32, f32),
}

/// A line from one task to another, as the points it passes through.
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    /// The task it leaves, as an index into the tasks.
    pub from: usize,
    /// The task it reaches.
    pub to: usize,
    /// Where it leaves, each place it crosses a column it skips, and where it arrives. Always left to right.
    pub points: Vec<(f32, f32)>,
}

/// The picture: where everything goes, and how big it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Each task, in the order given.
    pub nodes: Vec<Node>,
    /// Each dependency.
    pub edges: Vec<Edge>,
    /// How wide the picture is.
    pub width: f32,
    /// How tall.
    pub height: f32,
}

/// Measurements, in whatever unit the picture is drawn in.
#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    /// A task's width.
    pub node_w: f32,
    /// A task's height.
    pub node_h: f32,
    /// The space between columns, where lines run.
    pub gap_x: f32,
    /// The space between rows.
    pub gap_y: f32,
    /// The space around the whole picture.
    pub margin: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            node_w: 176.0,
            node_h: 58.0,
            gap_x: 52.0,
            gap_y: 14.0,
            margin: 14.0,
        }
    }
}

/// What a line stands for, from the states at its two ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Both ends are finished: work that has been handed on.
    Settled,
    /// The task it leaves is finished and the one it reaches is working: the hand-over happening now.
    Live,
    /// The task it leaves has stopped short: what is downstream cannot start.
    Held,
    /// Nothing has passed along it yet: the task it leaves has not finished, or the one it reaches has not started.
    Waiting,
}

pub use devd_runline::{is_done, is_stuck, span, QUIET_AFTER};

fn rules(tasks: &[Task]) -> Vec<devd_runline::Task> {
    tasks
        .iter()
        .map(|t| devd_runline::Task {
            key: t.key.clone(),
            role: t.role.clone(),
            state: t.state.clone(),
            depends_on: t.depends_on.clone(),
            quiet_secs: t.quiet_secs,
        })
        .collect()
}

/// How far along a run is: the tasks finished, and the tasks there are. See [`devd_runline::progress`].
pub fn progress(tasks: &[Task]) -> (usize, usize) {
    devd_runline::progress(&rules(tasks))
}

/// What is happening in a run, in a sentence. See [`devd_runline::status_line`].
pub fn status_line(run_state: &str, tasks: &[Task]) -> String {
    devd_runline::status_line(run_state, &rules(tasks))
}

/// What the line from a task in `from` to a task in `to` means.
pub fn flow(from: &str, to: &str) -> Flow {
    if is_done(from) {
        // Finished upstream. A line is only settled once what it leads to has finished as well: until then it is a
        // hand-over still to come, or the one happening now.
        if is_done(to) {
            Flow::Settled
        } else if matches!(to, "running" | "waiting_approval") {
            Flow::Live
        } else {
            Flow::Waiting
        }
    } else if is_stuck(from) {
        Flow::Held
    } else {
        Flow::Waiting
    }
}

/// Lay out a run's tasks.
///
/// A task with nothing to wait for follows the planner when there is one: the plan is what started them.
pub fn layout(tasks: &[Task], m: Metrics) -> Layout {
    let n = tasks.len();
    if n == 0 {
        return Layout {
            nodes: Vec::new(),
            edges: Vec::new(),
            width: 0.0,
            height: 0.0,
        };
    }
    let index: HashMap<&str, usize> = tasks
        .iter()
        .enumerate()
        .map(|(i, t)| (t.key.as_str(), i))
        .collect();
    let planner = tasks.iter().position(|t| t.role == "planner");
    // What each task waits for, as indexes. A name that matches nothing is ignored rather than trusted.
    let mut waits: Vec<Vec<usize>> = tasks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let mut found: Vec<usize> = t
                .depends_on
                .iter()
                .filter_map(|k| index.get(k.as_str()).copied())
                .filter(|&d| d != i)
                .collect();
            found.sort_unstable();
            found.dedup();
            found
        })
        .collect();
    if let Some(p) = planner {
        for (i, w) in waits.iter_mut().enumerate() {
            if i != p && w.is_empty() {
                w.push(p);
            }
        }
    }

    // Columns: one past the furthest of what a task waits for. A cycle cannot be drawn left to right, so a task
    // that is part of one is placed as if the edge that closes it were not there.
    let mut column = vec![usize::MAX; n];
    fn settle(
        i: usize,
        waits: &[Vec<usize>],
        column: &mut [usize],
        visiting: &mut Vec<bool>,
    ) -> usize {
        if column[i] != usize::MAX {
            return column[i];
        }
        if visiting[i] {
            return 0;
        }
        visiting[i] = true;
        let c = waits[i]
            .iter()
            .map(|&d| settle(d, waits, column, visiting) + 1)
            .max()
            .unwrap_or(0);
        visiting[i] = false;
        column[i] = c;
        c
    }
    let mut visiting = vec![false; n];
    for i in 0..n {
        settle(i, &waits, &mut column, &mut visiting);
    }
    // After a cycle was broken, an edge may still point backwards; those are dropped from the drawing.
    let edges_in: Vec<(usize, usize)> = (0..n)
        .flat_map(|to| waits[to].iter().map(move |&from| (from, to)))
        .filter(|&(from, to)| column[from] < column[to])
        .collect();

    // Lanes: a line that skips columns gets a stand-in in each column it skips, ordered with the real tasks.
    // Items 0..n are the tasks; the rest are stand-ins, each remembering which line it carries.
    let columns = column.iter().copied().max().unwrap_or(0) + 1;
    let mut slot_column: Vec<usize> = column.clone();
    let mut chains: Vec<Vec<usize>> = Vec::new();
    for &(from, to) in &edges_in {
        let mut chain = vec![from];
        for c in column[from] + 1..column[to] {
            slot_column.push(c);
            chain.push(slot_column.len() - 1);
        }
        chain.push(to);
        chains.push(chain);
    }
    let slots = slot_column.len();
    let mut up: Vec<Vec<usize>> = vec![Vec::new(); slots];
    let mut down: Vec<Vec<usize>> = vec![Vec::new(); slots];
    for chain in &chains {
        for pair in chain.windows(2) {
            down[pair[0]].push(pair[1]);
            up[pair[1]].push(pair[0]);
        }
    }

    // Order within columns: begin in the order given, then sweep left and right, putting each item at the average
    // position of its neighbours, which pulls lines straight and keeps them from crossing.
    let mut order: Vec<Vec<usize>> = vec![Vec::new(); columns];
    for (slot, &c) in slot_column.iter().enumerate() {
        order[c].push(slot);
    }
    let mut position = vec![0f32; slots];
    let refresh = |order: &Vec<Vec<usize>>, position: &mut Vec<f32>| {
        for col in order {
            for (at, &slot) in col.iter().enumerate() {
                position[slot] = at as f32;
            }
        }
    };
    refresh(&order, &mut position);
    for sweep in 0..8 {
        let columns_in_order: Vec<usize> = if sweep % 2 == 0 {
            (1..columns).collect()
        } else {
            (0..columns.saturating_sub(1)).rev().collect()
        };
        for c in columns_in_order {
            let neighbours = |slot: usize| {
                if sweep % 2 == 0 {
                    &up[slot]
                } else {
                    &down[slot]
                }
            };
            let mut keyed: Vec<(f32, usize, usize)> = order[c]
                .iter()
                .enumerate()
                .map(|(at, &slot)| {
                    let near = neighbours(slot);
                    let centre = if near.is_empty() {
                        at as f32
                    } else {
                        near.iter().map(|&s| position[s]).sum::<f32>() / near.len() as f32
                    };
                    (centre, at, slot)
                })
                .collect();
            keyed.sort_by(|a, b| {
                a.0.partial_cmp(&b.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.1.cmp(&b.1))
            });
            order[c] = keyed.into_iter().map(|(_, _, slot)| slot).collect();
            refresh(&order, &mut position);
        }
    }

    // Coordinates. Each column is centred on the tallest one, so a long chain reads as a spine.
    let row_h = m.node_h + m.gap_y;
    let tallest = order.iter().map(Vec::len).max().unwrap_or(1) as f32;
    let inner_h = tallest * row_h - m.gap_y;
    let mut center = vec![(0f32, 0f32); slots];
    for (c, col) in order.iter().enumerate() {
        let x = m.margin + c as f32 * (m.node_w + m.gap_x) + m.node_w / 2.0;
        let used = col.len() as f32 * row_h - m.gap_y;
        let top = m.margin + (inner_h - used) / 2.0;
        for (at, &slot) in col.iter().enumerate() {
            center[slot] = (x, top + at as f32 * row_h + m.node_h / 2.0);
        }
    }
    let half = m.node_w / 2.0;
    let edges = chains
        .iter()
        .map(|chain| {
            let from = chain[0];
            let to = *chain.last().unwrap_or(&from);
            let mut points = vec![(center[from].0 + half, center[from].1)];
            // Through each column it skips the line is level, on the lane, from one side of the column to the other.
            for &lane in &chain[1..chain.len() - 1] {
                points.push((center[lane].0 - half, center[lane].1));
                points.push((center[lane].0 + half, center[lane].1));
            }
            points.push((center[to].0 - half, center[to].1));
            Edge { from, to, points }
        })
        .collect();
    Layout {
        nodes: (0..n)
            .map(|i| Node {
                task: i,
                column: column[i],
                center: center[i],
            })
            .collect(),
        edges,
        width: m.margin * 2.0
            + columns as f32 * m.node_w
            + (columns.saturating_sub(1)) as f32 * m.gap_x,
        height: m.margin * 2.0 + inner_h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(key: &str, role: &str, state: &str, deps: &[&str]) -> Task {
        Task {
            key: key.into(),
            role: role.into(),
            title: key.into(),
            state: state.into(),
            thread: None,
            depends_on: deps.iter().map(|d| d.to_string()).collect(),
            quiet_secs: None,
            attempts: 1,
        }
    }

    fn column_of(l: &Layout, tasks: &[Task], key: &str) -> usize {
        l.nodes[tasks.iter().position(|t| t.key == key).unwrap()].column
    }

    #[test]
    fn a_task_sits_one_column_past_what_it_waits_for_and_every_line_runs_forward() {
        let tasks = vec![
            t("plan", "planner", "done", &[]),
            t("design", "architect", "done", &[]),
            t("cron", "implementer", "running", &["design"]),
            t("store", "backend", "running", &["design"]),
            t("scheduler", "implementer", "pending", &["cron", "store"]),
        ];
        let l = layout(&tasks, Metrics::default());
        assert_eq!(column_of(&l, &tasks, "plan"), 0);
        assert_eq!(
            column_of(&l, &tasks, "design"),
            1,
            "what has nothing to wait for follows the plan"
        );
        assert_eq!(column_of(&l, &tasks, "cron"), 2);
        assert_eq!(column_of(&l, &tasks, "store"), 2);
        assert_eq!(column_of(&l, &tasks, "scheduler"), 3);
        for edge in &l.edges {
            for pair in edge.points.windows(2) {
                assert!(pair[1].0 >= pair[0].0, "a line never doubles back");
            }
        }
    }

    #[test]
    fn a_line_that_skips_columns_runs_on_a_lane_clear_of_the_tasks_in_them() {
        // docs waits for cli (column 3) and for design (column 1): its line from design crosses columns 2 and 3.
        let tasks = vec![
            t("design", "architect", "done", &[]),
            t("a", "implementer", "done", &["design"]),
            t("b", "implementer", "done", &["a"]),
            t("docs", "docs", "pending", &["b", "design"]),
        ];
        let m = Metrics::default();
        let l = layout(&tasks, m);
        let long = l
            .edges
            .iter()
            .find(|e| tasks[e.from].key == "design" && tasks[e.to].key == "docs")
            .expect("the line exists");
        assert!(
            long.points.len() > 2,
            "it is carried through the columns it skips"
        );
        for &(x, y) in &long.points[1..long.points.len() - 1] {
            for node in &l.nodes {
                let (cx, cy) = node.center;
                let inside = x > cx - m.node_w / 2.0 + 0.5
                    && x < cx + m.node_w / 2.0 - 0.5
                    && (y - cy).abs() < m.node_h / 2.0;
                assert!(!inside, "no point of a line lies inside a task");
            }
        }
    }

    #[test]
    fn tasks_in_a_column_do_not_overlap_and_the_picture_holds_them_all() {
        let tasks = vec![
            t("design", "architect", "done", &[]),
            t("a", "implementer", "running", &["design"]),
            t("b", "backend", "running", &["design"]),
            t("c", "qa", "pending", &["design"]),
        ];
        let m = Metrics::default();
        let l = layout(&tasks, m);
        let mut centres: Vec<(usize, f32)> =
            l.nodes.iter().map(|n| (n.column, n.center.1)).collect();
        centres.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for pair in centres.windows(2) {
            if pair[0].0 == pair[1].0 {
                assert!(
                    pair[1].1 - pair[0].1 >= m.node_h,
                    "two tasks in one column are a task apart"
                );
            }
        }
        for n in &l.nodes {
            assert!(
                n.center.0 + m.node_w / 2.0 <= l.width && n.center.1 + m.node_h / 2.0 <= l.height
            );
            assert!(n.center.0 - m.node_w / 2.0 >= 0.0 && n.center.1 - m.node_h / 2.0 >= 0.0);
        }
    }

    #[test]
    fn two_independent_chains_do_not_cross() {
        let tasks = vec![
            t("a1", "implementer", "done", &[]),
            t("b1", "backend", "done", &[]),
            t("a2", "implementer", "done", &["a1"]),
            t("b2", "backend", "done", &["b1"]),
            t("a3", "qa", "done", &["a2"]),
            t("b3", "docs", "done", &["b2"]),
        ];
        let l = layout(&tasks, Metrics::default());
        let y = |k: &str| {
            l.nodes[tasks.iter().position(|t| t.key == k).unwrap()]
                .center
                .1
        };
        assert_eq!(
            y("a1") < y("b1"),
            y("a3") < y("b3"),
            "the chains keep their order"
        );
        assert_eq!(y("a1") < y("b1"), y("a2") < y("b2"));
    }

    #[test]
    fn a_dependency_on_nothing_or_on_itself_is_ignored_and_a_cycle_is_still_drawn() {
        let tasks = vec![
            t("a", "implementer", "pending", &["ghost", "a"]),
            t("b", "qa", "pending", &["c"]),
            t("c", "qa", "pending", &["b"]),
        ];
        let l = layout(&tasks, Metrics::default());
        assert_eq!(l.nodes.len(), 3, "nothing is lost, nothing loops for ever");
        assert!(layout(&[], Metrics::default()).nodes.is_empty());
    }

    #[test]
    fn what_a_line_means_follows_the_states_at_its_ends() {
        assert_eq!(flow("done", "done"), Flow::Settled);
        assert_eq!(flow("done", "running"), Flow::Live);
        assert_eq!(
            flow("done", "pending"),
            Flow::Waiting,
            "finished upstream is not yet handed on"
        );
        assert_eq!(flow("blocked", "pending"), Flow::Held);
        assert_eq!(flow("running", "pending"), Flow::Waiting);
        assert_eq!(flow("pending", "pending"), Flow::Waiting);
    }

    #[test]
    fn the_real_plan_of_a_run_lays_out_in_nine_columns_or_fewer() {
        let tasks = vec![
            t("plan", "planner", "done", &[]),
            t("design", "architect", "skipped", &[]),
            t("cron", "implementer", "running", &["design"]),
            t("store", "backend", "running", &["design"]),
            t("scheduler", "implementer", "pending", &["cron", "store"]),
            t("api", "backend", "pending", &["scheduler", "store"]),
            t("cli", "implementer", "pending", &["api"]),
            t("qa", "qa", "pending", &["cli", "cron", "store"]),
            t("security", "security", "pending", &["api", "scheduler"]),
            t("fixes", "implementer", "pending", &["security"]),
            t("docs", "docs", "pending", &["cli", "fixes"]),
            t("review", "reviewer", "pending", &["qa", "fixes", "docs"]),
        ];
        let l = layout(&tasks, Metrics::default());
        assert_eq!(l.nodes.iter().map(|n| n.column).max(), Some(8));
        assert_eq!(
            l.edges.len(),
            1 + 1 + 1 + 2 + 2 + 1 + 3 + 2 + 1 + 2 + 3,
            "every dependency has its line, and the plan feeds the design"
        );
    }
}
