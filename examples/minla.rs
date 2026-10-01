#!/usr/bin/env -S watchexec -r rust-script
//! ```cargo
//! [dependencies]
//! lissajous = { path = ".." }
//! cubecl = { version = "0.9.0", default-features = false, features = ["wgpu", "std"] }
//! egui = "0.33"
//! egui_plot = "0.34"
//! ```

use cubecl::prelude::*;
use cubecl::server::Handle;
use cubecl::wgpu::{WgpuDevice, WgpuRuntime};
use egui::{Color32, ColorImage, DragValue, Vec2};
use egui_plot::{Legend, Line, Plot, PlotPoints};
use std::collections::{hash_map::DefaultHasher, HashSet};
use std::f32::consts::TAU;
use std::hash::{Hash, Hasher};
use std::sync::{
    mpsc::{self, TryRecvError},
    Mutex,
};

use lissajous::cards::DEFAULT_CARD_PADDING;
use lissajous::prelude::*;
use lissajous::themes::{self, GorbieToggleButtonStyle};

const NODE_NAMES: [&str; 10] = [
    "source", "parse", "ast", "types", "solver", "layout", "render", "widgets", "cache", "io",
];

const PRESET_EDGES: [(usize, usize); 14] = [
    (0, 1),
    (1, 2),
    (2, 3),
    (3, 4),
    (4, 5),
    (5, 6),
    (6, 7),
    (0, 9),
    (9, 8),
    (8, 2),
    (8, 6),
    (7, 6),
    (1, 8),
    (3, 5),
];

const DEFAULT_NODE_COUNT: usize = NODE_NAMES.len();
const DEFAULT_EDGE_COUNT: usize = PRESET_EDGES.len();
const MAX_NODE_COUNT: usize = 4096;
const MAX_EDGE_COUNT: usize = 65_536;
const SEED_MIX: u32 = 0x9E37_79B9;
const LCG_A: u32 = 1_664_525;
const LCG_C: u32 = 1_013_904_223;

const MIN_BATCH_SIZE: u32 = 1;
const DEFAULT_BATCH_SIZE: u32 = MIN_BATCH_SIZE;
const MAX_BATCH_SIZE: u32 = 4096;
const DEFAULT_ANNEAL_STEPS: u32 = 1000;
const MIN_ANNEAL_STEPS: u32 = 1;
const MAX_ANNEAL_STEPS: u32 = 20_000;
const INV_U32_MAX_PLUS1: f32 = 1.0 / 4_294_967_296.0;
const DEFAULT_ANNEAL_TEMP: f32 = 8.0;
const MIN_ANNEAL_TEMP: f32 = 0.001;
const DEFAULT_TARGET_ACCEPTANCE: f32 = 0.3;
const RESEED_PLATEAU_STEPS: u32 = 12_000;
const CHAIN_TUNE_INTERVAL: u32 = 4;
const CHAIN_TUNE_TOLERANCE: f64 = 0.02;
const CHAIN_TUNE_STEP_MAX: u32 = 64;
const MAX_HISTORY: usize = 200;
const STRESS_HISTORY: usize = 360;
const STRESS_MAX_WIDTH: usize = 512;
const STRESS_VIEW_HEIGHT: f32 = STRESS_HISTORY as f32;
const AUTO_RUN_PULSE_HZ: f32 = 0.7;
const AUTO_RUN_PULSE_BASE: f32 = 0.14;
const AUTO_RUN_PULSE_RANGE: f32 = 0.08;

fn auto_run_pulse(ui: &egui::Ui) -> (Color32, Color32) {
    let style = GorbieToggleButtonStyle::from(ui.style().as_ref());
    let time = ui.input(|input| input.time) as f32;
    let wave = (time * AUTO_RUN_PULSE_HZ * TAU).sin();
    let mix = (AUTO_RUN_PULSE_BASE + AUTO_RUN_PULSE_RANGE * wave).clamp(0.0, 1.0);
    let fill = themes::blend(style.fill, style.accent, mix * 0.35);
    let light = themes::blend(style.led_on, Color32::WHITE, mix);
    (fill, light)
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum GraphPattern {
    Preset,
    Line,
    Ring,
    Star,
    Random,
}

impl GraphPattern {
    fn label(self) -> &'static str {
        match self {
            GraphPattern::Preset => "Preset",
            GraphPattern::Line => "Line",
            GraphPattern::Ring => "Ring",
            GraphPattern::Star => "Star",
            GraphPattern::Random => "Random",
        }
    }

    fn all() -> [GraphPattern; 5] {
        [
            GraphPattern::Preset,
            GraphPattern::Line,
            GraphPattern::Ring,
            GraphPattern::Star,
            GraphPattern::Random,
        ]
    }
}

#[derive(Clone, Debug)]
struct GraphConfig {
    pattern: GraphPattern,
    node_count: usize,
    edge_count: usize,
    seed: u64,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self {
            pattern: GraphPattern::Preset,
            node_count: DEFAULT_NODE_COUNT,
            edge_count: DEFAULT_EDGE_COUNT,
            seed: 1,
        }
    }
}

#[derive(Clone, Debug)]
struct GraphData {
    node_count: usize,
    edge_count: usize,
    edges: Vec<(usize, usize)>,
    edges_flat: Vec<u32>,
    adj_offsets: Vec<u32>,
    adj_list: Vec<u32>,
    id: u64,
}

#[derive(Clone, Debug)]
struct BatchResult {
    graph_id: u64,
    best_cost: u32,
    best_order: Vec<usize>,
    batch_size: usize,
    elapsed_ms: u128,
    error: Option<String>,
}

impl BatchResult {
    fn idle() -> Self {
        Self {
            graph_id: 0,
            best_cost: u32::MAX,
            best_order: Vec::new(),
            batch_size: 0,
            elapsed_ms: 0,
            error: None,
        }
    }

    fn ok(
        graph_id: u64,
        best_cost: u32,
        best_order: Vec<usize>,
        batch_size: usize,
        elapsed_ms: u128,
    ) -> Self {
        Self {
            graph_id,
            best_cost,
            best_order,
            batch_size,
            elapsed_ms,
            error: None,
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            graph_id: 0,
            best_cost: u32::MAX,
            best_order: Vec::new(),
            batch_size: 0,
            elapsed_ms: 0,
            error: Some(message.into()),
        }
    }
}

#[derive(Clone, Debug)]
struct BatchRequest {
    batch_size: usize,
    seed: u64,
    graph: GraphData,
}

#[derive(Clone, Debug)]
struct AnnealResult {
    graph_id: u64,
    best_cost: u32,
    reseeded: u32,
    batch_size: usize,
    steps: u32,
    elapsed_ms: u128,
    stress_row: Option<Vec<u32>>,
    change_row: Option<Vec<u32>>,
    touched_active: bool,
    error: Option<String>,
}

impl AnnealResult {
    fn idle() -> Self {
        Self {
            graph_id: 0,
            best_cost: u32::MAX,
            reseeded: 0,
            batch_size: 0,
            steps: 0,
            elapsed_ms: 0,
            stress_row: None,
            change_row: None,
            touched_active: false,
            error: None,
        }
    }

    fn ok(
        graph_id: u64,
        best_cost: u32,
        reseeded: u32,
        batch_size: usize,
        steps: u32,
        elapsed_ms: u128,
        stress_row: Option<Vec<u32>>,
        change_row: Option<Vec<u32>>,
        touched_active: bool,
    ) -> Self {
        Self {
            graph_id,
            best_cost,
            reseeded,
            batch_size,
            steps,
            elapsed_ms,
            stress_row,
            change_row,
            touched_active,
            error: None,
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            graph_id: 0,
            best_cost: u32::MAX,
            reseeded: 0,
            batch_size: 0,
            steps: 0,
            elapsed_ms: 0,
            stress_row: None,
            change_row: None,
            touched_active: false,
            error: Some(message.into()),
        }
    }
}

#[derive(Clone, Debug)]
struct AnnealRequest {
    batch_size: usize,
    steps: u32,
    seed: u64,
    reset: bool,
    initial_temp: f32,
    temp_floor: f32,
    reseed_plateau_steps: u32,
    cooling_adjust: f32,
    capture_stress: bool,
    capture_touched: bool,
    graph: GraphData,
}

struct BatchRunner {
    sender: mpsc::Sender<BatchRequest>,
    receiver: Mutex<mpsc::Receiver<BatchResult>>,
    in_flight: bool,
}

impl BatchRunner {
    fn new_wgpu() -> Self {
        spawn_worker(wgpu_worker_loop)
    }

    fn spawn(&mut self, request: BatchRequest) -> Result<(), BatchResult> {
        if self.in_flight {
            return Ok(());
        }
        if self.sender.send(request).is_err() {
            return Err(BatchResult::failed("worker disconnected"));
        }
        self.in_flight = true;
        Ok(())
    }

    fn poll(&mut self) -> Option<BatchResult> {
        let receiver = self
            .receiver
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match receiver.try_recv() {
            Ok(batch) => {
                self.in_flight = false;
                Some(batch)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.in_flight = false;
                Some(BatchResult::failed("worker disconnected"))
            }
        }
    }

    fn is_running(&self) -> bool {
        self.in_flight
    }
}

struct AnnealRunner {
    sender: mpsc::Sender<AnnealRequest>,
    receiver: Mutex<mpsc::Receiver<AnnealResult>>,
    in_flight: bool,
}

impl AnnealRunner {
    fn new_wgpu() -> Self {
        spawn_anneal_worker(wgpu_anneal_worker_loop)
    }

    fn spawn(&mut self, request: AnnealRequest) -> Result<(), AnnealResult> {
        if self.in_flight {
            return Ok(());
        }
        if self.sender.send(request).is_err() {
            return Err(AnnealResult::failed("worker disconnected"));
        }
        self.in_flight = true;
        Ok(())
    }

    fn poll(&mut self) -> Option<AnnealResult> {
        let receiver = self
            .receiver
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match receiver.try_recv() {
            Ok(batch) => {
                self.in_flight = false;
                Some(batch)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.in_flight = false;
                Some(AnnealResult::failed("worker disconnected"))
            }
        }
    }

    fn is_running(&self) -> bool {
        self.in_flight
    }
}

struct BatchRunnerState<R: Runtime> {
    _device: R::Device,
    client: ComputeClient<R>,
    edges_handle: Handle,
    graph_id: u64,
    node_count: usize,
    edge_count: usize,
}

impl<R: Runtime> BatchRunnerState<R> {
    fn new(device: R::Device, graph: &GraphData) -> Self {
        let client = R::client(&device);
        let edges_handle = client.create_from_slice(u32::as_bytes(&graph.edges_flat));
        Self {
            _device: device,
            client,
            edges_handle,
            graph_id: graph.id,
            node_count: graph.node_count,
            edge_count: graph.edge_count,
        }
    }

    fn run(&mut self, graph: &GraphData, batch_size: usize, seed: u64) -> BatchResult {
        if batch_size == 0 {
            return BatchResult::failed("batch size must be > 0");
        }
        if graph.node_count == 0 {
            return BatchResult::failed("graph must have at least one node");
        }

        self.sync_graph(graph);

        let start = std::time::Instant::now();
        let seed32 = seed_to_u32(seed);

        let output_handle = self.client.empty(batch_size * std::mem::size_of::<u32>());
        let orders_len = match batch_size.checked_mul(self.node_count) {
            Some(len) if len > 0 => len,
            _ => return BatchResult::failed("batch too large for graph size"),
        };
        let orders_handle = self.client.empty(orders_len * std::mem::size_of::<u32>());
        let positions_handle = self.client.empty(orders_len * std::mem::size_of::<u32>());

        unsafe {
            minla_cost_kernel::launch::<R>(
                &self.client,
                CubeCount::new_1d(batch_size as u32),
                CubeDim::new_1d(1),
                ArrayArg::from_raw_parts(self.edges_handle.clone(), graph.edges_flat.len()),
                self.node_count as u32,
                self.edge_count as u32,
                seed32,
                ArrayArg::from_raw_parts(output_handle.clone(), batch_size),
                ArrayArg::from_raw_parts(orders_handle.clone(), orders_len),
                ArrayArg::from_raw_parts(positions_handle.clone(), orders_len),
            );
        }

        let costs_bytes = self.client.read_one(output_handle).expect("gpu readback");
        let costs = u32::from_bytes(&costs_bytes);

        let mut best_cost = u32::MAX;
        let mut best_idx = 0usize;
        for (idx, cost) in costs.iter().enumerate() {
            if *cost < best_cost {
                best_cost = *cost;
                best_idx = idx;
            }
        }

        let order_stride = self.node_count * std::mem::size_of::<u32>();
        let offset_start = (best_idx * order_stride) as u64;
        let offset_end = (batch_size.saturating_sub(best_idx + 1) * order_stride) as u64;
        let order_handle = orders_handle
            .clone()
            .offset_start(offset_start)
            .offset_end(offset_end);
        let order_bytes = self.client.read_one(order_handle).expect("gpu readback");
        let order = u32::from_bytes(&order_bytes);
        let best_order = order
            .iter()
            .take(self.node_count)
            .map(|&val| val as usize)
            .collect();

        BatchResult::ok(
            graph.id,
            best_cost,
            best_order,
            batch_size,
            start.elapsed().as_millis(),
        )
    }

    fn sync_graph(&mut self, graph: &GraphData) {
        if self.graph_id == graph.id {
            return;
        }
        self.edges_handle = self
            .client
            .create_from_slice(u32::as_bytes(&graph.edges_flat));
        self.graph_id = graph.id;
        self.node_count = graph.node_count;
        self.edge_count = graph.edge_count;
    }
}

struct AnnealRunnerState<R: Runtime> {
    _device: R::Device,
    client: ComputeClient<R>,
    edges_handle: Handle,
    adj_offsets_handle: Handle,
    adj_list_handle: Handle,
    graph_id: u64,
    node_count: usize,
    edge_count: usize,
    batch_size: usize,
    best_cost: u32,
    best_version: u32,
    orders_handle: Option<Handle>,
    positions_handle: Option<Handle>,
    best_orders_handle: Option<Handle>,
    current_costs_handle: Option<Handle>,
    best_orders_costs_handle: Option<Handle>,
    temp_floor_handle: Option<Handle>,
    rng_states_handle: Option<Handle>,
    stagnant_steps_handle: Option<Handle>,
    reseeded_handle: Option<Handle>,
    seed_versions_handle: Option<Handle>,
    stress_handle: Option<Handle>,
    best_order_handle: Option<Handle>,
    best_positions_handle: Option<Handle>,
    prev_best_order: Option<Vec<u32>>,
}

impl<R: Runtime> AnnealRunnerState<R> {
    fn new(device: R::Device, graph: &GraphData) -> Self {
        let client = R::client(&device);
        let edges_handle = client.create_from_slice(u32::as_bytes(&graph.edges_flat));
        let adj_offsets_handle = client.create_from_slice(u32::as_bytes(&graph.adj_offsets));
        let adj_list_handle = client.create_from_slice(u32::as_bytes(&graph.adj_list));
        Self {
            _device: device,
            client,
            edges_handle,
            adj_offsets_handle,
            adj_list_handle,
            graph_id: graph.id,
            node_count: graph.node_count,
            edge_count: graph.edge_count,
            batch_size: 0,
            best_cost: u32::MAX,
            best_version: 0,
            orders_handle: None,
            positions_handle: None,
            best_orders_handle: None,
            current_costs_handle: None,
            best_orders_costs_handle: None,
            temp_floor_handle: None,
            rng_states_handle: None,
            stagnant_steps_handle: None,
            reseeded_handle: None,
            seed_versions_handle: None,
            stress_handle: None,
            best_order_handle: None,
            best_positions_handle: None,
            prev_best_order: None,
        }
    }

    fn run(&mut self, request: &AnnealRequest) -> AnnealResult {
        if request.batch_size == 0 {
            return AnnealResult::failed("batch size must be > 0");
        }
        if request.graph.node_count == 0 {
            return AnnealResult::failed("graph must have at least one node");
        }
        if let Err(error) = self.ensure_state(request) {
            return error;
        }

        let start = std::time::Instant::now();
        let steps = request.steps.max(1);
        let cooling_adjust = request.cooling_adjust.max(0.0001).min(0.05);
        let reheat_steps = request.reseed_plateau_steps.max(1);

        let orders_handle = self.orders_handle.as_ref().expect("orders handle");
        let positions_handle = self.positions_handle.as_ref().expect("positions handle");
        let current_costs_handle = self
            .current_costs_handle
            .as_ref()
            .expect("current costs handle");
        let best_orders_handle = self
            .best_orders_handle
            .as_ref()
            .expect("best orders handle");
        let best_orders_costs_handle = self
            .best_orders_costs_handle
            .as_ref()
            .expect("best orders cost handle");
        let temp_floor_handle = self.temp_floor_handle.as_ref().expect("temp floor handle");
        let rng_states_handle = self.rng_states_handle.as_ref().expect("rng states handle");
        let stagnant_steps_handle = self
            .stagnant_steps_handle
            .as_ref()
            .expect("stagnant steps handle");
        let reseeded_handle = self.reseeded_handle.as_ref().expect("reseeded handle");
        let seed_versions_handle = self
            .seed_versions_handle
            .as_ref()
            .expect("seed versions handle");
        let orders_len = request.batch_size * self.node_count;

        unsafe {
            minla_sa_kernel::launch::<R>(
                &self.client,
                CubeCount::new_1d(request.batch_size as u32),
                CubeDim::new_1d(1),
                ArrayArg::from_raw_parts(
                    self.adj_offsets_handle.clone(),
                    request.graph.adj_offsets.len(),
                ),
                ArrayArg::from_raw_parts(
                    self.adj_list_handle.clone(),
                    request.graph.adj_list.len(),
                ),
                self.node_count as u32,
                steps,
                cooling_adjust,
                reheat_steps,
                ArrayArg::from_raw_parts(orders_handle.clone(), orders_len),
                ArrayArg::from_raw_parts(positions_handle.clone(), orders_len),
                ArrayArg::from_raw_parts(current_costs_handle.clone(), request.batch_size),
                ArrayArg::from_raw_parts(best_orders_handle.clone(), orders_len),
                ArrayArg::from_raw_parts(best_orders_costs_handle.clone(), request.batch_size),
                ArrayArg::from_raw_parts(temp_floor_handle.clone(), request.batch_size),
                ArrayArg::from_raw_parts(rng_states_handle.clone(), request.batch_size),
                ArrayArg::from_raw_parts(stagnant_steps_handle.clone(), request.batch_size),
            );
        }

        let best_orders_costs_bytes = self
            .client
            .read_one(best_orders_costs_handle.clone())
            .expect("gpu readback");
        let best_orders_costs = u32::from_bytes(&best_orders_costs_bytes).to_vec();
        let mut batch_best_cost = u32::MAX;
        let mut batch_best_idx = 0usize;
        if !best_orders_costs.is_empty() {
            for (idx, cost) in best_orders_costs.iter().enumerate() {
                if *cost < batch_best_cost {
                    batch_best_cost = *cost;
                    batch_best_idx = idx;
                }
            }
        }
        if batch_best_cost != u32::MAX && batch_best_cost < self.best_cost {
            self.best_cost = batch_best_cost;
            self.best_version = self.best_version.saturating_add(1);
        }

        let reset_cap = request.initial_temp.max(MIN_ANNEAL_TEMP);
        let reset_floor = request.temp_floor.max(MIN_ANNEAL_TEMP).min(reset_cap);
        let stale_steps =
            if request.batch_size > 1 && batch_best_cost != u32::MAX && self.node_count > 0 {
                reheat_steps
            } else {
                0
            };
        unsafe {
            minla_sa_reseed_kernel::launch::<R>(
                &self.client,
                CubeCount::new_1d(request.batch_size as u32),
                CubeDim::new_1d(1),
                self.node_count as u32,
                batch_best_idx as u32,
                batch_best_cost,
                stale_steps,
                self.best_version,
                reset_floor,
                ArrayArg::from_raw_parts(orders_handle.clone(), orders_len),
                ArrayArg::from_raw_parts(positions_handle.clone(), orders_len),
                ArrayArg::from_raw_parts(best_orders_handle.clone(), orders_len),
                ArrayArg::from_raw_parts(current_costs_handle.clone(), request.batch_size),
                ArrayArg::from_raw_parts(best_orders_costs_handle.clone(), request.batch_size),
                ArrayArg::from_raw_parts(temp_floor_handle.clone(), request.batch_size),
                ArrayArg::from_raw_parts(stagnant_steps_handle.clone(), request.batch_size),
                ArrayArg::from_raw_parts(reseeded_handle.clone(), request.batch_size),
                ArrayArg::from_raw_parts(seed_versions_handle.clone(), request.batch_size),
            );
        }

        let reseeded_bytes = self
            .client
            .read_one(reseeded_handle.clone())
            .expect("gpu readback");
        let reseeded_flags = u32::from_bytes(&reseeded_bytes).to_vec();
        let reseeded = reseeded_flags.iter().filter(|&&value| value != 0).count() as u32;

        let mut stress_row = None;
        let mut change_row = None;
        if request.capture_stress {
            let stress_handle = self.stress_handle.as_ref().expect("stress handle");
            let best_order_handle = self.best_order_handle.as_ref().expect("best order handle");
            let best_positions_handle = self
                .best_positions_handle
                .as_ref()
                .expect("best positions handle");
            unsafe {
                minla_select_best_order_kernel::launch::<R>(
                    &self.client,
                    CubeCount::new_1d(self.node_count as u32),
                    CubeDim::new_1d(1),
                    self.node_count as u32,
                    batch_best_idx as u32,
                    ArrayArg::from_raw_parts(best_orders_handle.clone(), orders_len),
                    ArrayArg::from_raw_parts(best_order_handle.clone(), self.node_count),
                );
            }

            if request.capture_touched {
                let order_bytes = self
                    .client
                    .read_one(best_order_handle.clone())
                    .expect("gpu readback");
                let mut best_order = u32::from_bytes(&order_bytes).to_vec();
                best_order.truncate(self.node_count);
                if let Some(prev) = self.prev_best_order.as_ref() {
                    if prev.len() == best_order.len() {
                        let mut changes = vec![0u32; self.node_count];
                        for (idx, (&curr, &prev)) in best_order.iter().zip(prev).enumerate() {
                            changes[idx] = (curr != prev) as u32;
                        }
                        change_row = Some(changes);
                    }
                }
                self.prev_best_order = Some(best_order);
            }

            unsafe {
                minla_positions_from_order_kernel::launch::<R>(
                    &self.client,
                    CubeCount::new_1d(self.node_count as u32),
                    CubeDim::new_1d(1),
                    self.node_count as u32,
                    ArrayArg::from_raw_parts(best_order_handle.clone(), self.node_count),
                    ArrayArg::from_raw_parts(best_positions_handle.clone(), self.node_count),
                );

                minla_stress_kernel::launch::<R>(
                    &self.client,
                    CubeCount::new_1d(self.node_count as u32),
                    CubeDim::new_1d(1),
                    ArrayArg::from_raw_parts(
                        self.adj_offsets_handle.clone(),
                        request.graph.adj_offsets.len(),
                    ),
                    ArrayArg::from_raw_parts(
                        self.adj_list_handle.clone(),
                        request.graph.adj_list.len(),
                    ),
                    self.node_count as u32,
                    ArrayArg::from_raw_parts(best_positions_handle.clone(), self.node_count),
                    1u32,
                    ArrayArg::from_raw_parts(stress_handle.clone(), self.node_count),
                );
            }
            let stress_bytes = self
                .client
                .read_one(stress_handle.clone())
                .expect("gpu readback");
            let mut row = u32::from_bytes(&stress_bytes).to_vec();
            row.truncate(self.node_count);
            stress_row = Some(row);
        }

        AnnealResult::ok(
            request.graph.id,
            batch_best_cost,
            reseeded,
            request.batch_size,
            steps,
            start.elapsed().as_millis(),
            stress_row,
            change_row,
            request.capture_touched,
        )
    }

    fn sync_graph(&mut self, graph: &GraphData) -> bool {
        if self.graph_id == graph.id {
            return false;
        }
        self.edges_handle = self
            .client
            .create_from_slice(u32::as_bytes(&graph.edges_flat));
        self.adj_offsets_handle = self
            .client
            .create_from_slice(u32::as_bytes(&graph.adj_offsets));
        self.adj_list_handle = self
            .client
            .create_from_slice(u32::as_bytes(&graph.adj_list));
        self.graph_id = graph.id;
        self.node_count = graph.node_count;
        self.edge_count = graph.edge_count;
        true
    }

    fn ensure_state(&mut self, request: &AnnealRequest) -> Result<(), AnnealResult> {
        let graph_changed = self.sync_graph(&request.graph);
        let has_state = self.orders_handle.is_some();
        let needs_reset = request.reset || graph_changed || !has_state;

        if needs_reset {
            self.prev_best_order = None;
            self.best_cost = u32::MAX;
            self.best_version = 0;
        }

        if self.stress_handle.is_none() || graph_changed {
            let stress_len = self.node_count.max(1);
            let stress_bytes = stress_len * std::mem::size_of::<u32>();
            self.stress_handle = Some(self.client.empty(stress_bytes));
        }
        if self.best_order_handle.is_none() || graph_changed {
            let order_len = self.node_count.max(1);
            let order_bytes = order_len * std::mem::size_of::<u32>();
            self.best_order_handle = Some(self.client.empty(order_bytes));
        }
        if self.best_positions_handle.is_none() || graph_changed {
            let positions_len = self.node_count.max(1);
            let positions_bytes = positions_len * std::mem::size_of::<u32>();
            self.best_positions_handle = Some(self.client.empty(positions_bytes));
        }

        if !needs_reset && request.batch_size <= self.batch_size {
            return Ok(());
        }

        let batch_size = request.batch_size;
        let order_len = match batch_size.checked_mul(self.node_count) {
            Some(len) if len > 0 => len,
            _ => return Err(AnnealResult::failed("batch too large for graph size")),
        };
        let bytes_u32 = std::mem::size_of::<u32>();
        let bytes_f32 = std::mem::size_of::<f32>();
        let order_bytes = match order_len.checked_mul(bytes_u32) {
            Some(bytes) => bytes,
            None => return Err(AnnealResult::failed("order buffer too large")),
        };
        let batch_bytes_u32 = match batch_size.checked_mul(bytes_u32) {
            Some(bytes) => bytes,
            None => return Err(AnnealResult::failed("batch buffer too large")),
        };
        let batch_bytes_f32 = match batch_size.checked_mul(bytes_f32) {
            Some(bytes) => bytes,
            None => return Err(AnnealResult::failed("batch buffer too large")),
        };

        let grow = !needs_reset && batch_size > self.batch_size;
        let old_batch_size = self.batch_size;
        let old_orders_handle = if grow {
            Some(self.orders_handle.as_ref().expect("orders handle").clone())
        } else {
            None
        };
        let old_positions_handle = if grow {
            Some(
                self.positions_handle
                    .as_ref()
                    .expect("positions handle")
                    .clone(),
            )
        } else {
            None
        };
        let old_best_orders_handle = if grow {
            Some(
                self.best_orders_handle
                    .as_ref()
                    .expect("best orders handle")
                    .clone(),
            )
        } else {
            None
        };
        let old_current_costs_handle = if grow {
            Some(
                self.current_costs_handle
                    .as_ref()
                    .expect("current costs handle")
                    .clone(),
            )
        } else {
            None
        };
        let old_best_orders_costs_handle = if grow {
            Some(
                self.best_orders_costs_handle
                    .as_ref()
                    .expect("best orders cost handle")
                    .clone(),
            )
        } else {
            None
        };
        let old_temp_floor_handle = if grow {
            Some(
                self.temp_floor_handle
                    .as_ref()
                    .expect("temp floor handle")
                    .clone(),
            )
        } else {
            None
        };
        let old_rng_states_handle = if grow {
            Some(
                self.rng_states_handle
                    .as_ref()
                    .expect("rng states handle")
                    .clone(),
            )
        } else {
            None
        };
        let old_stagnant_steps_handle = if grow {
            Some(
                self.stagnant_steps_handle
                    .as_ref()
                    .expect("stagnant steps handle")
                    .clone(),
            )
        } else {
            None
        };
        let old_seed_versions_handle = if grow {
            Some(
                self.seed_versions_handle
                    .as_ref()
                    .expect("seed versions handle")
                    .clone(),
            )
        } else {
            None
        };

        self.orders_handle = Some(self.client.empty(order_bytes));
        self.positions_handle = Some(self.client.empty(order_bytes));
        self.best_orders_handle = Some(self.client.empty(order_bytes));
        self.current_costs_handle = Some(self.client.empty(batch_bytes_u32));
        self.best_orders_costs_handle = Some(self.client.empty(batch_bytes_u32));
        self.temp_floor_handle = Some(self.client.empty(batch_bytes_f32));
        self.rng_states_handle = Some(self.client.empty(batch_bytes_u32));
        self.stagnant_steps_handle = Some(self.client.empty(batch_bytes_u32));
        self.reseeded_handle = Some(self.client.empty(batch_bytes_u32));
        self.seed_versions_handle = Some(self.client.empty(batch_bytes_u32));
        self.batch_size = batch_size;

        let seed32 = seed_to_u32(request.seed);
        let initial_temp = request.initial_temp.max(MIN_ANNEAL_TEMP);
        let floor_temp = request.temp_floor.max(MIN_ANNEAL_TEMP).min(initial_temp);

        let orders_handle = self.orders_handle.as_ref().expect("orders handle");
        let positions_handle = self.positions_handle.as_ref().expect("positions handle");
        let best_orders_handle = self
            .best_orders_handle
            .as_ref()
            .expect("best orders handle");
        let current_costs_handle = self
            .current_costs_handle
            .as_ref()
            .expect("current costs handle");
        let best_orders_costs_handle = self
            .best_orders_costs_handle
            .as_ref()
            .expect("best orders cost handle");
        let temp_floor_handle = self.temp_floor_handle.as_ref().expect("temp floor handle");
        let rng_states_handle = self.rng_states_handle.as_ref().expect("rng states handle");
        let stagnant_steps_handle = self
            .stagnant_steps_handle
            .as_ref()
            .expect("stagnant steps handle");
        let seed_versions_handle = self
            .seed_versions_handle
            .as_ref()
            .expect("seed versions handle");

        unsafe {
            minla_sa_init_kernel::launch::<R>(
                &self.client,
                CubeCount::new_1d(batch_size as u32),
                CubeDim::new_1d(1),
                ArrayArg::from_raw_parts(self.edges_handle.clone(), request.graph.edges_flat.len()),
                self.node_count as u32,
                self.edge_count as u32,
                seed32,
                initial_temp,
                floor_temp,
                ArrayArg::from_raw_parts(orders_handle.clone(), order_len),
                ArrayArg::from_raw_parts(positions_handle.clone(), order_len),
                ArrayArg::from_raw_parts(best_orders_handle.clone(), order_len),
                ArrayArg::from_raw_parts(current_costs_handle.clone(), batch_size),
                ArrayArg::from_raw_parts(best_orders_costs_handle.clone(), batch_size),
                ArrayArg::from_raw_parts(temp_floor_handle.clone(), batch_size),
                ArrayArg::from_raw_parts(rng_states_handle.clone(), batch_size),
                ArrayArg::from_raw_parts(stagnant_steps_handle.clone(), batch_size),
                ArrayArg::from_raw_parts(seed_versions_handle.clone(), batch_size),
            );
        }

        if grow {
            let old_batch_size = old_batch_size;
            let old_order_len = match old_batch_size.checked_mul(self.node_count) {
                Some(len) if len > 0 => len,
                _ => return Err(AnnealResult::failed("batch too large for graph size")),
            };
            let old_orders_handle = old_orders_handle.expect("old orders handle");
            let old_positions_handle = old_positions_handle.expect("old positions handle");
            let old_best_orders_handle = old_best_orders_handle.expect("old best orders handle");
            let old_current_costs_handle =
                old_current_costs_handle.expect("old current costs handle");
            let old_best_orders_costs_handle =
                old_best_orders_costs_handle.expect("old best orders cost handle");
            let old_temp_floor_handle = old_temp_floor_handle.expect("old temp floor handle");
            let old_rng_states_handle = old_rng_states_handle.expect("old rng states handle");
            let old_stagnant_steps_handle =
                old_stagnant_steps_handle.expect("old stagnant steps handle");
            let old_seed_versions_handle =
                old_seed_versions_handle.expect("old seed versions handle");

            unsafe {
                minla_sa_copy_kernel::launch::<R>(
                    &self.client,
                    CubeCount::new_1d(batch_size as u32),
                    CubeDim::new_1d(1),
                    self.node_count as u32,
                    old_batch_size as u32,
                    ArrayArg::from_raw_parts(old_orders_handle.clone(), old_order_len),
                    ArrayArg::from_raw_parts(old_positions_handle.clone(), old_order_len),
                    ArrayArg::from_raw_parts(old_best_orders_handle.clone(), old_order_len),
                    ArrayArg::from_raw_parts(old_current_costs_handle.clone(), old_batch_size),
                    ArrayArg::from_raw_parts(old_best_orders_costs_handle.clone(), old_batch_size),
                    ArrayArg::from_raw_parts(old_temp_floor_handle.clone(), old_batch_size),
                    ArrayArg::from_raw_parts(old_rng_states_handle.clone(), old_batch_size),
                    ArrayArg::from_raw_parts(old_stagnant_steps_handle.clone(), old_batch_size),
                    ArrayArg::from_raw_parts(old_seed_versions_handle.clone(), old_batch_size),
                    ArrayArg::from_raw_parts(orders_handle.clone(), order_len),
                    ArrayArg::from_raw_parts(positions_handle.clone(), order_len),
                    ArrayArg::from_raw_parts(best_orders_handle.clone(), order_len),
                    ArrayArg::from_raw_parts(current_costs_handle.clone(), batch_size),
                    ArrayArg::from_raw_parts(best_orders_costs_handle.clone(), batch_size),
                    ArrayArg::from_raw_parts(temp_floor_handle.clone(), batch_size),
                    ArrayArg::from_raw_parts(rng_states_handle.clone(), batch_size),
                    ArrayArg::from_raw_parts(stagnant_steps_handle.clone(), batch_size),
                    ArrayArg::from_raw_parts(seed_versions_handle.clone(), batch_size),
                );
            }
        }

        Ok(())
    }
}

fn spawn_worker(
    worker: fn(mpsc::Receiver<BatchRequest>, mpsc::Sender<BatchResult>),
) -> BatchRunner {
    let (request_tx, request_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    std::thread::spawn(move || worker(request_rx, result_tx));
    BatchRunner {
        sender: request_tx,
        receiver: Mutex::new(result_rx),
        in_flight: false,
    }
}

fn spawn_anneal_worker(
    worker: fn(mpsc::Receiver<AnnealRequest>, mpsc::Sender<AnnealResult>),
) -> AnnealRunner {
    let (request_tx, request_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    std::thread::spawn(move || worker(request_rx, result_tx));
    AnnealRunner {
        sender: request_tx,
        receiver: Mutex::new(result_rx),
        in_flight: false,
    }
}

fn build_wgpu_runner(graph: &GraphData) -> BatchRunnerState<WgpuRuntime> {
    let device = WgpuDevice::default();
    BatchRunnerState::new(device, graph)
}

fn build_wgpu_anneal_runner(graph: &GraphData) -> AnnealRunnerState<WgpuRuntime> {
    let device = WgpuDevice::default();
    AnnealRunnerState::new(device, graph)
}

fn process_request<R: Runtime>(
    runner: &mut Option<BatchRunnerState<R>>,
    request: BatchRequest,
    init: fn(&GraphData) -> BatchRunnerState<R>,
) -> BatchResult {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if runner.is_none() {
            *runner = Some(init(&request.graph));
        }
        runner.as_mut().expect("runner missing").run(
            &request.graph,
            request.batch_size,
            request.seed,
        )
    }));
    match result {
        Ok(batch) => batch,
        Err(_) => {
            *runner = None;
            BatchResult::failed("cubecl failed to initialize wgpu backend")
        }
    }
}

fn process_anneal_request<R: Runtime>(
    runner: &mut Option<AnnealRunnerState<R>>,
    request: AnnealRequest,
    init: fn(&GraphData) -> AnnealRunnerState<R>,
) -> AnnealResult {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if runner.is_none() {
            *runner = Some(init(&request.graph));
        }
        runner.as_mut().expect("runner missing").run(&request)
    }));
    match result {
        Ok(batch) => batch,
        Err(_) => {
            *runner = None;
            AnnealResult::failed("cubecl failed to initialize wgpu backend")
        }
    }
}

fn wgpu_worker_loop(requests: mpsc::Receiver<BatchRequest>, results: mpsc::Sender<BatchResult>) {
    let mut runner: Option<BatchRunnerState<WgpuRuntime>> = None;
    for request in requests {
        let batch = process_request(&mut runner, request, build_wgpu_runner);
        let _ = results.send(batch);
    }
}

fn wgpu_anneal_worker_loop(
    requests: mpsc::Receiver<AnnealRequest>,
    results: mpsc::Sender<AnnealResult>,
) {
    let mut runner: Option<AnnealRunnerState<WgpuRuntime>> = None;
    for request in requests {
        let batch = process_anneal_request(&mut runner, request, build_wgpu_anneal_runner);
        let _ = results.send(batch);
    }
}

#[derive(Clone, Debug)]
struct BatchHistory {
    run: usize,
    batch_cost: u32,
    best_cost: u32,
    elapsed_ms: u128,
}

#[derive(Clone, Debug)]
struct BestResult {
    cost: u32,
    order: Vec<usize>,
}

impl BestResult {
    fn new() -> Self {
        Self {
            cost: u32::MAX,
            order: Vec::new(),
        }
    }
}

struct MinlaState {
    runner_wgpu: BatchRunner,
    last: BatchResult,
    best: BestResult,
    history: Vec<BatchHistory>,
    auto_run: bool,
    total_samples: usize,
    runs: usize,
    graph_id: u64,
}

impl MinlaState {
    fn new() -> Self {
        Self {
            runner_wgpu: BatchRunner::new_wgpu(),
            last: BatchResult::idle(),
            best: BestResult::new(),
            history: Vec::new(),
            auto_run: false,
            total_samples: 0,
            runs: 0,
            graph_id: 0,
        }
    }

    fn poll_runners(&mut self) {
        if let Some(batch) = self.runner_wgpu.poll() {
            if batch.error.is_some() {
                self.last = batch;
                return;
            }
            if batch.graph_id == self.graph_id {
                self.record_batch(&batch);
                self.last = batch;
            }
        }
    }

    fn record_batch(&mut self, batch: &BatchResult) {
        if batch.error.is_some() || batch.batch_size == 0 {
            return;
        }
        self.runs += 1;
        self.total_samples += batch.batch_size;
        if batch.best_cost < self.best.cost {
            self.best.cost = batch.best_cost;
            self.best.order = batch.best_order.clone();
        }
        self.history.push(BatchHistory {
            run: self.runs,
            batch_cost: batch.best_cost,
            best_cost: self.best.cost,
            elapsed_ms: batch.elapsed_ms,
        });
        if self.history.len() > MAX_HISTORY {
            let drop = self.history.len() - MAX_HISTORY;
            self.history.drain(0..drop);
        }
    }

    fn sync_graph(&mut self, graph: &GraphData) {
        if self.graph_id == graph.id {
            return;
        }
        self.graph_id = graph.id;
        self.reset_metrics();
    }

    fn reset_metrics(&mut self) {
        self.last = BatchResult::idle();
        self.best = BestResult::new();
        self.history.clear();
        self.runs = 0;
        self.total_samples = 0;
    }
}

#[derive(Clone, Debug)]
struct Config {
    batch_size: u32,
    seed: u64,
    graph: GraphConfig,
    graph_data: GraphData,
    difficulty: f32,
    use_difficulty: bool,
    auto_batch: bool,
    target_batch_ms: u32,
}

impl Default for Config {
    fn default() -> Self {
        let mut graph = GraphConfig::default();
        let difficulty = 0.35;
        let use_difficulty = true;
        if use_difficulty {
            apply_difficulty(&mut graph, difficulty);
        }
        normalize_graph_config(&mut graph);
        let graph_data = build_graph(&graph);
        Self {
            batch_size: DEFAULT_BATCH_SIZE,
            seed: 42,
            graph,
            graph_data,
            difficulty,
            use_difficulty,
            auto_batch: true,
            target_batch_ms: 60,
        }
    }
}

impl Config {
    fn sync_graph(&mut self) {
        if self.use_difficulty {
            apply_difficulty(&mut self.graph, self.difficulty);
        }
        normalize_graph_config(&mut self.graph);
        let id = graph_id(&self.graph);
        if self.graph_data.id != id {
            self.graph_data = build_graph(&self.graph);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct LcgRng {
    state: u64,
}

impl LcgRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        (self.state >> 32) as u32
    }

    fn gen_range(&mut self, upper: usize) -> usize {
        if upper == 0 {
            return 0;
        }
        (self.next_u32() as usize) % upper
    }
}

fn graph_id(config: &GraphConfig) -> u64 {
    let mut hasher = DefaultHasher::new();
    config.pattern.hash(&mut hasher);
    config.node_count.hash(&mut hasher);
    config.edge_count.hash(&mut hasher);
    config.seed.hash(&mut hasher);
    hasher.finish()
}

fn max_edges(node_count: usize) -> usize {
    node_count.saturating_sub(1) * node_count / 2
}

fn normalize_graph_config(config: &mut GraphConfig) {
    if config.pattern == GraphPattern::Preset {
        config.node_count = DEFAULT_NODE_COUNT;
        config.edge_count = DEFAULT_EDGE_COUNT;
        return;
    }
    config.node_count = config.node_count.clamp(2, MAX_NODE_COUNT);
    let max_edges = max_edges(config.node_count).min(MAX_EDGE_COUNT);
    config.edge_count = config.edge_count.min(max_edges);
}

fn base_edges_for_pattern(pattern: GraphPattern, node_count: usize) -> usize {
    match pattern {
        GraphPattern::Preset => PRESET_EDGES.len(),
        GraphPattern::Line => node_count.saturating_sub(1),
        GraphPattern::Ring => {
            if node_count > 2 {
                node_count
            } else {
                node_count.saturating_sub(1)
            }
        }
        GraphPattern::Star => node_count.saturating_sub(1),
        GraphPattern::Random => 0,
    }
}

fn apply_difficulty(config: &mut GraphConfig, difficulty: f32) {
    let d = difficulty.clamp(0.0, 1.0);
    let pattern = if d < 0.25 {
        GraphPattern::Line
    } else if d < 0.5 {
        GraphPattern::Ring
    } else if d < 0.75 {
        GraphPattern::Star
    } else {
        GraphPattern::Random
    };
    config.pattern = pattern;

    let min_nodes = 4usize;
    let max_nodes = MAX_NODE_COUNT;
    let nodes = min_nodes + ((max_nodes - min_nodes) as f32 * d.powf(1.4)).round() as usize;
    config.node_count = nodes.clamp(2, MAX_NODE_COUNT);

    let max_edges = max_edges(config.node_count).min(MAX_EDGE_COUNT);
    let density = 0.15 + 0.7 * d.powf(1.6);
    let target_edges = (max_edges as f32 * density).round() as usize;
    let base_edges = base_edges_for_pattern(pattern, config.node_count);
    config.edge_count = target_edges.max(base_edges).min(max_edges);
}

fn add_edge(
    edges: &mut Vec<(usize, usize)>,
    seen: &mut HashSet<(usize, usize)>,
    u: usize,
    v: usize,
) -> bool {
    if u == v {
        return false;
    }
    let (a, b) = if u < v { (u, v) } else { (v, u) };
    if seen.insert((a, b)) {
        edges.push((a, b));
        true
    } else {
        false
    }
}

fn build_graph(config: &GraphConfig) -> GraphData {
    let mut edges = Vec::new();
    let mut seen = HashSet::new();
    let node_count = config.node_count;

    match config.pattern {
        GraphPattern::Preset => {
            for &(u, v) in PRESET_EDGES.iter() {
                add_edge(&mut edges, &mut seen, u, v);
            }
        }
        GraphPattern::Line => {
            for i in 0..node_count.saturating_sub(1) {
                add_edge(&mut edges, &mut seen, i, i + 1);
            }
        }
        GraphPattern::Ring => {
            for i in 0..node_count.saturating_sub(1) {
                add_edge(&mut edges, &mut seen, i, i + 1);
            }
            if node_count > 2 {
                add_edge(&mut edges, &mut seen, 0, node_count - 1);
            }
        }
        GraphPattern::Star => {
            if node_count > 1 {
                for i in 1..node_count {
                    add_edge(&mut edges, &mut seen, 0, i);
                }
            }
        }
        GraphPattern::Random => {}
    }

    if edges.len() > config.edge_count {
        edges.truncate(config.edge_count);
    } else if edges.len() < config.edge_count && node_count > 1 {
        let max_edges = max_edges(node_count).min(MAX_EDGE_COUNT);
        let mut rng = LcgRng::new(config.seed ^ 0x9E37_79B9_7F4A_7C15);
        while edges.len() < config.edge_count && edges.len() < max_edges {
            let u = rng.gen_range(node_count);
            let mut v = rng.gen_range(node_count - 1);
            if v >= u {
                v += 1;
            }
            add_edge(&mut edges, &mut seen, u, v);
        }
    }

    let mut edges_flat = Vec::with_capacity(edges.len() * 2);
    for (u, v) in &edges {
        edges_flat.push(*u as u32);
        edges_flat.push(*v as u32);
    }

    let mut degrees = vec![0usize; node_count];
    for &(u, v) in &edges {
        degrees[u] += 1;
        degrees[v] += 1;
    }
    let mut adj_offsets = Vec::with_capacity(node_count + 1);
    adj_offsets.push(0u32);
    for i in 0..node_count {
        let next = adj_offsets[i] + degrees[i] as u32;
        adj_offsets.push(next);
    }
    let total_adj = *adj_offsets.last().unwrap_or(&0) as usize;
    let mut adj_list = vec![0u32; total_adj];
    let mut cursor: Vec<usize> = adj_offsets.iter().map(|&offset| offset as usize).collect();
    for &(u, v) in &edges {
        let idx = cursor[u];
        adj_list[idx] = v as u32;
        cursor[u] += 1;
        let idx = cursor[v];
        adj_list[idx] = u as u32;
        cursor[v] += 1;
    }

    let id = graph_id(config);
    GraphData {
        node_count,
        edge_count: edges.len(),
        edges,
        edges_flat,
        adj_offsets,
        adj_list,
        id,
    }
}

#[derive(Clone, Debug)]
struct AnnealConfig {
    steps_per_batch: u32,
    initial_temp: f32,
    seed: u64,
    temp_floor: f32,
    cooling_adjust: f32,
}

impl Default for AnnealConfig {
    fn default() -> Self {
        Self {
            steps_per_batch: DEFAULT_ANNEAL_STEPS,
            initial_temp: DEFAULT_ANNEAL_TEMP,
            seed: 7,
            temp_floor: DEFAULT_ANNEAL_TEMP,
            cooling_adjust: 0.002,
        }
    }
}

fn estimate_initial_temp(graph: &GraphData, seed: u64) -> f32 {
    let node_count = graph.node_count.max(2);
    let mut order: Vec<usize> = (0..node_count).collect();
    let base_cost = cost_cpu(&order, graph);
    let mut rng = LcgRng::new(seed ^ 0x9E37_79B9_7F4A_7C15);
    let sample_count = node_count.min(64).max(8);
    let mut sum_delta = 0u64;
    let mut count = 0u32;

    for _ in 0..sample_count {
        let i = rng.gen_range(node_count);
        let mut j = rng.gen_range(node_count - 1);
        if j >= i {
            j += 1;
        }
        order.swap(i, j);
        let candidate_cost = cost_cpu(&order, graph);
        order.swap(i, j);
        if candidate_cost > base_cost {
            sum_delta += (candidate_cost - base_cost) as u64;
            count += 1;
        }
    }

    let avg_delta = if count > 0 {
        sum_delta as f32 / count as f32
    } else {
        1.0
    };
    let target = DEFAULT_TARGET_ACCEPTANCE.clamp(0.05, 0.95);
    let denom = -target.ln();
    let temp = if denom > 0.0 {
        avg_delta / denom
    } else {
        avg_delta
    };
    temp.clamp(0.1, 100_000.0)
}

fn auto_tune_anneal_config(config: &mut AnnealConfig, graph: &GraphData) {
    config.steps_per_batch = MIN_ANNEAL_STEPS;
    config.cooling_adjust = 0.002;
    config.seed = graph.id ^ 0xD1B5_4A32_D192_ED03;
    config.initial_temp = estimate_initial_temp(graph, config.seed);
    config.temp_floor = (config.initial_temp * 0.25).max(MIN_ANNEAL_TEMP);
}

fn adjust_to_target(current: u32, elapsed_ms: u128, target_ms: u32, min: u32, max: u32) -> u32 {
    if elapsed_ms == 0 {
        return current.saturating_mul(2).clamp(min, max);
    }
    let target = target_ms.max(1) as f64;
    let elapsed = elapsed_ms as f64;
    let ratio = target / elapsed;
    if (0.9..=1.1).contains(&ratio) {
        return current.clamp(min, max);
    }
    let factor = ratio.clamp(0.5, 2.0);
    let next = (current as f64 * factor).round() as u32;
    next.clamp(min, max)
}

fn adjust_batch_size(current: u32, elapsed_ms: u128, target_ms: u32) -> u32 {
    adjust_to_target(
        current,
        elapsed_ms,
        target_ms,
        MIN_BATCH_SIZE,
        MAX_BATCH_SIZE,
    )
}

fn adjust_steps_per_batch(current: u32, elapsed_ms: u128, target_ms: u32) -> u32 {
    adjust_to_target(
        current,
        elapsed_ms,
        target_ms,
        MIN_ANNEAL_STEPS,
        MAX_ANNEAL_STEPS,
    )
}

fn stress_color(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let value = (t * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_gray(value)
}

fn downsample_row(row: &[u32], target: usize) -> Vec<u32> {
    downsample_row_impl(row, target, |slice| {
        let sum: u64 = slice.iter().map(|&value| value as u64).sum();
        let count = slice.len().max(1) as u64;
        (sum / count) as u32
    })
}

fn downsample_mask(row: &[u32], target: usize) -> Vec<u8> {
    downsample_row_impl(row, target, |slice| {
        let active = slice.iter().filter(|&&value| value > 0).count() as f32;
        let ratio = active / slice.len().max(1) as f32;
        (ratio * 255.0).round() as u8
    })
}

fn downsample_row_impl<T>(
    row: &[u32],
    target: usize,
    mut sample: impl FnMut(&[u32]) -> T,
) -> Vec<T> {
    if row.is_empty() || target == 0 {
        return Vec::new();
    }
    if row.len() <= target {
        return row
            .iter()
            .map(|value| sample(std::slice::from_ref(value)))
            .collect();
    }
    let mut out = Vec::with_capacity(target);
    for idx in 0..target {
        let start = idx * row.len() / target;
        let end = (idx + 1) * row.len() / target;
        out.push(sample(&row[start..end]));
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StressView {
    Tension,
    Touched,
}

struct StressHeatmap {
    width: usize,
    height: usize,
    head: usize,
    rows: Vec<u32>,
    change_rows: Vec<u8>,
    max_value: f32,
    texture: Option<egui::TextureHandle>,
    touched_texture: Option<egui::TextureHandle>,
}

impl StressHeatmap {
    fn new() -> Self {
        Self {
            width: 0,
            height: 0,
            head: 0,
            rows: Vec::new(),
            change_rows: Vec::new(),
            max_value: 0.0,
            texture: None,
            touched_texture: None,
        }
    }

    fn clear(&mut self) {
        self.width = 0;
        self.height = 0;
        self.head = 0;
        self.rows.clear();
        self.change_rows.clear();
        self.max_value = 0.0;
        self.texture = None;
        self.touched_texture = None;
    }

    fn ensure_tension_size(&mut self, width: usize, height: usize, ctx: &egui::Context) {
        if self.width == width && self.height == height && self.texture.is_some() {
            return;
        }
        self.width = width;
        self.height = height;
        self.head = 0;
        self.rows = vec![0; width.saturating_mul(height)];
        self.change_rows = vec![0; width.saturating_mul(height)];
        self.max_value = 0.0;
        let image = ColorImage::new([width, height], vec![Color32::BLACK; width * height]);
        self.texture = Some(ctx.load_texture("minla_stress", image, egui::TextureOptions::NEAREST));
        self.touched_texture = None;
    }

    fn ensure_touched_size(&mut self, ctx: &egui::Context) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        if self.touched_texture.is_some() {
            return;
        }
        let touched = ColorImage::new(
            [self.width, self.height],
            vec![Color32::BLACK; self.width * self.height],
        );
        self.touched_texture = Some(ctx.load_texture(
            "minla_stress_touched",
            touched,
            egui::TextureOptions::NEAREST,
        ));
    }

    fn push_row(
        &mut self,
        row: &[u32],
        change_row: Option<&[u32]>,
        update_touched: bool,
        ctx: &egui::Context,
    ) {
        if row.is_empty() {
            return;
        }
        let width = row.len().min(STRESS_MAX_WIDTH);
        let height = STRESS_HISTORY.max(1);
        self.ensure_tension_size(width, height, ctx);
        let downsampled = downsample_row(row, self.width);
        if downsampled.len() != self.width {
            return;
        }
        let row_max = downsampled.iter().copied().max().unwrap_or(0) as f32;
        if self.max_value <= 0.0 {
            self.max_value = row_max.max(1.0);
        } else {
            self.max_value = (self.max_value * 0.98).max(row_max).max(1.0);
        }
        let offset = self.head * self.width;
        self.rows[offset..offset + self.width].copy_from_slice(&downsampled);
        if update_touched {
            self.ensure_touched_size(ctx);
            if let Some(change_row) = change_row {
                let downsampled_changes = downsample_mask(change_row, self.width);
                if downsampled_changes.len() == self.width {
                    self.change_rows[offset..offset + self.width]
                        .copy_from_slice(&downsampled_changes);
                } else {
                    self.change_rows[offset..offset + self.width].fill(0);
                }
            } else {
                self.change_rows[offset..offset + self.width].fill(0);
            }
        }
        self.head = (self.head + 1) % self.height;
        self.update_tension_texture(ctx);
        if update_touched {
            self.update_touched_texture(ctx);
        }
    }

    fn update_tension_texture(&mut self, ctx: &egui::Context) {
        let Some(texture) = &mut self.texture else {
            return;
        };
        let mut pixels = Vec::with_capacity(self.width * self.height);
        let max_value = self.max_value.max(1.0);
        let log_denom = (max_value + 1.0).ln();
        for row in 0..self.height {
            let src_row = (self.head + self.height - 1 - row) % self.height;
            let start = src_row * self.width;
            for col in 0..self.width {
                let value = self.rows[start + col] as f32;
                let t = if log_denom > 0.0 {
                    ((value + 1.0).ln() / log_denom).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                pixels.push(stress_color(t));
            }
        }
        let image = ColorImage::new([self.width, self.height], pixels);
        texture.set(image, egui::TextureOptions::NEAREST);
        ctx.request_repaint();
    }

    fn update_touched_texture(&mut self, ctx: &egui::Context) {
        let Some(touched_texture) = &mut self.touched_texture else {
            return;
        };
        let mut touched_pixels = Vec::with_capacity(self.width * self.height);
        for row in 0..self.height {
            let src_row = (self.head + self.height - 1 - row) % self.height;
            let start = src_row * self.width;
            for col in 0..self.width {
                let intensity = self.change_rows[start + col] as f32 / 255.0;
                let gray = (intensity * 255.0).round().clamp(0.0, 255.0) as u8;
                touched_pixels.push(Color32::from_gray(gray));
            }
        }
        let touched = ColorImage::new([self.width, self.height], touched_pixels);
        touched_texture.set(touched, egui::TextureOptions::NEAREST);
        ctx.request_repaint();
    }

    fn clear_touched(&mut self) {
        if !self.change_rows.is_empty() {
            self.change_rows.fill(0);
        }
        self.touched_texture = None;
    }

    fn show(&self, ui: &mut egui::Ui, view: StressView) {
        let texture = match view {
            StressView::Tension => self.texture.as_ref(),
            StressView::Touched => self.touched_texture.as_ref(),
        };
        let Some(texture) = texture else {
            match view {
                StressView::Tension => {
                    ui.label("Stress view will appear after the first batch.");
                }
                StressView::Touched => {
                    ui.label("Touched view will appear after one batch.");
                }
            };
            return;
        };
        let width = ui.available_width().max(1.0);
        let size = Vec2::new(width, STRESS_VIEW_HEIGHT);
        ui.add(
            egui::Image::from_texture(texture)
                .fit_to_exact_size(size)
                .maintain_aspect_ratio(false),
        );
    }
}

#[derive(Clone, Debug)]
struct AnnealHistory {
    run: usize,
    best_cost: u32,
    reseeded: u32,
    elapsed_ms: u128,
}

struct AnnealState {
    config: AnnealConfig,
    runner: AnnealRunner,
    last: AnnealResult,
    best_cost: u32,
    history: Vec<AnnealHistory>,
    auto_run: bool,
    runs: usize,
    total_steps: usize,
    total_chains: usize,
    total_elapsed_ms: u128,
    start_cost: Option<u32>,
    graph: GraphData,
    graph_id: u64,
    pending_reset: bool,
    force_reinit: bool,
    chain_count: u32,
    auto_chains: bool,
    chain_tune_batches: u32,
    chain_rate_ema: f64,
    chain_last_rate: f64,
    chain_step: u32,
    chain_direction: i32,
    stress: StressHeatmap,
    stress_view: StressView,
}

impl AnnealState {
    fn new() -> Self {
        let mut config = AnnealConfig::default();
        let mut graph_config = GraphConfig::default();
        normalize_graph_config(&mut graph_config);
        let graph = build_graph(&graph_config);
        auto_tune_anneal_config(&mut config, &graph);
        Self {
            config,
            runner: AnnealRunner::new_wgpu(),
            last: AnnealResult::idle(),
            best_cost: u32::MAX,
            history: Vec::new(),
            auto_run: false,
            runs: 0,
            total_steps: 0,
            total_chains: 0,
            total_elapsed_ms: 0,
            start_cost: None,
            graph_id: graph.id,
            graph,
            pending_reset: false,
            force_reinit: true,
            chain_count: MIN_BATCH_SIZE,
            auto_chains: true,
            chain_tune_batches: 0,
            chain_rate_ema: 0.0,
            chain_last_rate: 0.0,
            chain_step: 2,
            chain_direction: 1,
            stress: StressHeatmap::new(),
            stress_view: StressView::Tension,
        }
    }

    fn clear_run_state(&mut self) {
        self.last = AnnealResult::idle();
        self.best_cost = u32::MAX;
        self.history.clear();
        self.runs = 0;
        self.total_steps = 0;
        self.total_chains = 0;
        self.total_elapsed_ms = 0;
        self.start_cost = None;
        self.force_reinit = true;
        self.chain_tune_batches = 0;
        self.chain_rate_ema = 0.0;
        self.chain_last_rate = 0.0;
        self.chain_step = 2;
        self.chain_direction = 1;
        self.stress.clear();
        self.stress_view = StressView::Tension;
    }

    fn reset(&mut self) {
        auto_tune_anneal_config(&mut self.config, &self.graph);
        self.clear_run_state();
        self.pending_reset = false;
    }

    fn ensure_start_cost(&mut self) {
        if self.start_cost.is_some() {
            return;
        }
        if self.graph.node_count == 0 {
            self.start_cost = Some(0);
            return;
        }
        let baseline_order: Vec<usize> = (0..self.graph.node_count).collect();
        self.start_cost = Some(cost_cpu(&baseline_order, &self.graph));
    }

    fn sync_graph(&mut self, graph: &GraphData) {
        if self.graph_id == graph.id {
            return;
        }
        self.graph = graph.clone();
        self.graph_id = graph.id;
        self.pending_reset = true;
        self.clear_run_state();
    }

    fn maybe_reset(&mut self) {
        if self.pending_reset && !self.runner.is_running() {
            self.reset();
            self.pending_reset = false;
        }
    }

    fn poll_runner(&mut self, target_ms: u32, ctx: &egui::Context) {
        if let Some(batch) = self.runner.poll() {
            if batch.error.is_some() {
                self.last = batch;
                return;
            }
            if batch.graph_id == self.graph_id {
                if let Some(row) = batch.stress_row.as_deref() {
                    self.stress.push_row(
                        row,
                        batch.change_row.as_deref(),
                        batch.touched_active,
                        ctx,
                    );
                }
                self.record_batch(&batch, target_ms);
                self.last = batch;
            }
        }
    }

    fn record_batch(&mut self, batch: &AnnealResult, target_ms: u32) {
        if batch.steps == 0 || batch.graph_id != self.graph_id {
            return;
        }
        self.runs += 1;
        self.total_steps += batch.steps as usize * batch.batch_size;
        self.total_chains += batch.batch_size;
        self.total_elapsed_ms += batch.elapsed_ms;
        self.ensure_start_cost();
        if batch.best_cost < self.best_cost {
            self.best_cost = batch.best_cost;
        }
        self.history.push(AnnealHistory {
            run: self.runs,
            best_cost: self.best_cost,
            reseeded: batch.reseeded,
            elapsed_ms: batch.elapsed_ms,
        });
        if self.history.len() > MAX_HISTORY {
            let drop = self.history.len() - MAX_HISTORY;
            self.history.drain(0..drop);
        }
        if batch.elapsed_ms > 0 && batch.steps > 0 {
            self.config.steps_per_batch =
                adjust_steps_per_batch(batch.steps, batch.elapsed_ms, target_ms);
        }
        self.tune_chains(batch);
    }

    fn tune_chains(&mut self, batch: &AnnealResult) {
        if !self.auto_chains {
            return;
        }
        if batch.elapsed_ms == 0 || batch.steps == 0 {
            return;
        }
        let rate = batch.steps as f64 * batch.batch_size as f64 / batch.elapsed_ms as f64;
        if self.chain_rate_ema == 0.0 {
            self.chain_rate_ema = rate;
        } else {
            self.chain_rate_ema = self.chain_rate_ema * 0.8 + rate * 0.2;
        }
        self.chain_tune_batches += 1;
        if self.chain_tune_batches < CHAIN_TUNE_INTERVAL {
            return;
        }
        self.chain_tune_batches = 0;
        if self.chain_last_rate == 0.0 {
            self.chain_last_rate = self.chain_rate_ema;
            return;
        }

        let gain = (self.chain_rate_ema - self.chain_last_rate) / self.chain_last_rate;
        if gain.abs() < CHAIN_TUNE_TOLERANCE {
            self.chain_step = (self.chain_step / 2).max(1);
            self.chain_last_rate = self.chain_rate_ema;
            return;
        }
        if gain < 0.0 {
            self.chain_direction = -self.chain_direction;
            self.chain_step = (self.chain_step / 2).max(1);
        } else if self.chain_step < CHAIN_TUNE_STEP_MAX {
            self.chain_step = (self.chain_step.saturating_mul(2)).min(CHAIN_TUNE_STEP_MAX);
        }
        self.chain_last_rate = self.chain_rate_ema;

        let current = self.chain_count.max(1);
        let next = (current as i64 + self.chain_direction as i64 * self.chain_step as i64)
            .clamp(MIN_BATCH_SIZE as i64, MAX_BATCH_SIZE as i64) as u32;
        if next != current {
            self.chain_count = next;
        }
    }
}

#[cube(launch)]
fn minla_cost_kernel(
    edges: &Array<u32>,
    node_count: u32,
    edge_count: u32,
    seed: u32,
    output: &mut Array<u32>,
    orders: &mut Array<u32>,
    positions: &mut Array<u32>,
) {
    let candidate = ABSOLUTE_POS;
    let mut state = seed ^ candidate as u32;
    state = state * SEED_MIX;
    let node_count = node_count as usize;
    let edge_count = edge_count as usize;
    if node_count == 0 {
        output[candidate] = 0;
    } else {
        let base = candidate * node_count;
        for index in 0..node_count {
            orders[base + index] = index as u32;
        }

        if node_count > 1 {
            for i in 0..node_count {
                state = state * LCG_A + LCG_C;
                let remaining = node_count - i;
                let j = (state % remaining as u32) as usize + i;
                let left = base + i;
                let right = base + j;
                let tmp = orders[left];
                orders[left] = orders[right];
                orders[right] = tmp;
            }
        }

        for pos in 0..node_count {
            let node = orders[base + pos] as usize;
            positions[base + node] = pos as u32;
        }

        let mut cost = u32::new(0);
        for edge in 0..edge_count {
            let edge_index = edge * 2;
            let u = edges[edge_index] as usize;
            let v = edges[edge_index + 1] as usize;
            let pu = positions[base + u];
            let pv = positions[base + v];
            let diff = if pu > pv { pu - pv } else { pv - pu };
            cost += diff;
        }

        output[candidate] = cost;
    }
}

#[cube(launch)]
fn minla_sa_init_kernel(
    edges: &Array<u32>,
    node_count: u32,
    edge_count: u32,
    seed: u32,
    initial_temp: f32,
    floor_temp: f32,
    orders: &mut Array<u32>,
    positions: &mut Array<u32>,
    best_orders: &mut Array<u32>,
    current_costs: &mut Array<u32>,
    best_orders_costs: &mut Array<u32>,
    temp_floors: &mut Array<f32>,
    rng_states: &mut Array<u32>,
    stagnant_steps: &mut Array<u32>,
    seed_versions: &mut Array<u32>,
) {
    let candidate = ABSOLUTE_POS;
    let node_count = node_count as usize;
    let edge_count = edge_count as usize;
    let temp = initial_temp.max(MIN_ANNEAL_TEMP);
    let floor_temp = floor_temp.max(MIN_ANNEAL_TEMP).min(temp);
    if node_count == 0 {
        current_costs[candidate] = 0;
        best_orders_costs[candidate] = 0;
        temp_floors[candidate] = floor_temp;
        rng_states[candidate] = seed;
        stagnant_steps[candidate] = 0;
    } else {
        let mut state = seed ^ candidate as u32;
        state = state * SEED_MIX;
        let base = candidate * node_count;

        for index in 0..node_count {
            orders[base + index] = index as u32;
        }

        if node_count > 1 {
            for i in 0..node_count {
                state = state * LCG_A + LCG_C;
                let remaining = node_count - i;
                let j = (state % remaining as u32) as usize + i;
                let left = base + i;
                let right = base + j;
                let tmp = orders[left];
                orders[left] = orders[right];
                orders[right] = tmp;
            }
        }

        for pos in 0..node_count {
            let node = orders[base + pos] as usize;
            positions[base + node] = pos as u32;
        }

        let mut cost = u32::new(0);
        for edge in 0..edge_count {
            let edge_index = edge * 2;
            let u = edges[edge_index] as usize;
            let v = edges[edge_index + 1] as usize;
            let pu = positions[base + u];
            let pv = positions[base + v];
            let diff = if pu > pv { pu - pv } else { pv - pu };
            cost += diff;
        }

        current_costs[candidate] = cost;
        best_orders_costs[candidate] = cost;
        temp_floors[candidate] = floor_temp;
        rng_states[candidate] = state;
        stagnant_steps[candidate] = 0;
        seed_versions[candidate] = 0;

        for idx in 0..node_count {
            best_orders[base + idx] = orders[base + idx];
        }
    }
}

#[cube(launch)]
fn minla_sa_kernel(
    adj_offsets: &Array<u32>,
    adj_list: &Array<u32>,
    node_count: u32,
    steps: u32,
    cooling_adjust: f32,
    reheat_steps: u32,
    orders: &mut Array<u32>,
    positions: &mut Array<u32>,
    current_costs: &mut Array<u32>,
    best_orders: &mut Array<u32>,
    best_orders_costs: &mut Array<u32>,
    temp_floors: &mut Array<f32>,
    rng_states: &mut Array<u32>,
    stagnant_steps: &mut Array<u32>,
) {
    let candidate = ABSOLUTE_POS;
    let node_count = node_count as usize;
    let steps = steps as usize;
    if node_count == 0 {
        current_costs[candidate] = 0;
    } else {
        let base = candidate * node_count;
        let mut state = rng_states[candidate];
        let mut current_cost = current_costs[candidate];
        let mut temp_floor = temp_floors[candidate].max(MIN_ANNEAL_TEMP);
        let cooling_adjust = cooling_adjust.max(0.0001).min(0.05);
        let mut stagnant = stagnant_steps[candidate];
        let mut best_cost = best_orders_costs[candidate];
        let reheat_gain = (cooling_adjust * 4.0).max(0.001).min(0.05);
        let floor_decay = (1.0 - cooling_adjust * 2.0).max(0.90).min(0.9999);
        let reheat_steps = reheat_steps.max(1);
        if node_count > 1 && steps > 0 {
            for _ in 0..steps {
                let temperature = temp_floor;
                state = state * LCG_A + LCG_C;
                let i = (state % node_count as u32) as usize;
                state = state * LCG_A + LCG_C;
                let mut j = (state % (node_count as u32 - 1)) as usize;
                if j >= i {
                    j += 1;
                }

                let left = base + i;
                let right = base + j;
                let node_i = orders[left];
                let node_j = orders[right];
                orders[left] = node_j;
                orders[right] = node_i;
                positions[base + node_j as usize] = i as u32;
                positions[base + node_i as usize] = j as u32;

                let node_i_usize = node_i as usize;
                let node_j_usize = node_j as usize;
                let pos_i = i as u32;
                let pos_j = j as u32;
                let mut delta_cost = i32::new(0);

                let start_i = adj_offsets[node_i_usize] as usize;
                let end_i = adj_offsets[node_i_usize + 1] as usize;
                for idx in start_i..end_i {
                    let neighbor = adj_list[idx] as usize;
                    if neighbor != node_j_usize {
                        let pos_n = positions[base + neighbor];
                        let old = if pos_i > pos_n {
                            pos_i - pos_n
                        } else {
                            pos_n - pos_i
                        };
                        let new = if pos_j > pos_n {
                            pos_j - pos_n
                        } else {
                            pos_n - pos_j
                        };
                        delta_cost += new as i32 - old as i32;
                    }
                }

                let start_j = adj_offsets[node_j_usize] as usize;
                let end_j = adj_offsets[node_j_usize + 1] as usize;
                for idx in start_j..end_j {
                    let neighbor = adj_list[idx] as usize;
                    if neighbor != node_i_usize {
                        let pos_n = positions[base + neighbor];
                        let old = if pos_j > pos_n {
                            pos_j - pos_n
                        } else {
                            pos_n - pos_j
                        };
                        let new = if pos_i > pos_n {
                            pos_i - pos_n
                        } else {
                            pos_n - pos_i
                        };
                        delta_cost += new as i32 - old as i32;
                    }
                }

                let candidate_cost = (current_cost as i32 + delta_cost).max(0) as u32;

                let delta = candidate_cost as f32 - current_cost as f32;
                let mut accept = delta <= 0.0;
                if !accept && temperature > MIN_ANNEAL_TEMP {
                    let probability = (-delta / temperature).exp();
                    state = state * LCG_A + LCG_C;
                    let rand = state as f32 * INV_U32_MAX_PLUS1;
                    accept = rand < probability;
                }

                if accept {
                    current_cost = candidate_cost;
                    if candidate_cost < best_cost {
                        for idx in 0..node_count {
                            best_orders[base + idx] = orders[base + idx];
                        }
                        best_cost = candidate_cost;
                        stagnant = 0;
                        temp_floor = (temp_floor * floor_decay).max(MIN_ANNEAL_TEMP);
                    } else if stagnant < u32::MAX {
                        stagnant += 1;
                    }
                } else {
                    let left = base + i;
                    let right = base + j;
                    orders[left] = node_i;
                    orders[right] = node_j;
                    positions[base + node_i as usize] = i as u32;
                    positions[base + node_j as usize] = j as u32;
                    if stagnant < u32::MAX {
                        stagnant += 1;
                    }
                }
                if stagnant > 0 {
                    let ratio = stagnant as f32 / reheat_steps as f32;
                    let gain = reheat_gain * (ratio / (1.0 + ratio));
                    temp_floor = temp_floor * (1.0 + gain);
                }
            }
        }

        current_costs[candidate] = current_cost;
        best_orders_costs[candidate] = best_cost;
        temp_floors[candidate] = temp_floor;
        rng_states[candidate] = state;
        stagnant_steps[candidate] = stagnant;
    }
}

#[cube(launch)]
fn minla_positions_from_order_kernel(
    node_count: u32,
    order: &Array<u32>,
    positions: &mut Array<u32>,
) {
    let idx = ABSOLUTE_POS;
    let node_count = node_count as usize;
    if idx < node_count {
        let node = order[idx] as usize;
        positions[node] = idx as u32;
    }
}

#[cube(launch)]
fn minla_select_best_order_kernel(
    node_count: u32,
    best_idx: u32,
    best_orders: &Array<u32>,
    best_order_out: &mut Array<u32>,
) {
    let idx = ABSOLUTE_POS;
    let node_count = node_count as usize;
    if idx < node_count {
        let base = best_idx as usize * node_count;
        best_order_out[idx] = best_orders[base + idx];
    }
}

#[cube(launch)]
fn minla_stress_kernel(
    adj_offsets: &Array<u32>,
    adj_list: &Array<u32>,
    node_count: u32,
    positions: &Array<u32>,
    normalize: u32,
    stress_out: &mut Array<u32>,
) {
    let node = ABSOLUTE_POS;
    let node_count = node_count as usize;
    if node < node_count {
        let pos_u = positions[node] as u32;
        let start = adj_offsets[node] as usize;
        let end = adj_offsets[node + 1] as usize;
        let degree = (end - start) as u32;
        let mut sum = u32::new(0);
        for idx in start..end {
            let neighbor = adj_list[idx] as usize;
            let pos_v = positions[neighbor] as u32;
            sum += if pos_u > pos_v {
                pos_u - pos_v
            } else {
                pos_v - pos_u
            };
        }
        let stress = if normalize != 0 && degree > 0 {
            sum / degree
        } else {
            sum
        };
        stress_out[pos_u as usize] = stress;
    }
}

#[cube(launch)]
fn minla_sa_reseed_kernel(
    node_count: u32,
    best_idx: u32,
    best_cost: u32,
    stale_steps: u32,
    best_version: u32,
    reset_floor: f32,
    orders: &mut Array<u32>,
    positions: &mut Array<u32>,
    best_orders: &mut Array<u32>,
    current_costs: &mut Array<u32>,
    best_orders_costs: &mut Array<u32>,
    temp_floors: &mut Array<f32>,
    stagnant_steps: &mut Array<u32>,
    reseeded_flags: &mut Array<u32>,
    seed_versions: &mut Array<u32>,
) {
    let candidate = ABSOLUTE_POS;
    let node_count = node_count as usize;
    let best_idx = best_idx as usize;
    if candidate == best_idx {
        seed_versions[candidate] = best_version;
    }
    let should_reseed = stale_steps > 0
        && candidate != best_idx
        && stagnant_steps[candidate] >= stale_steps
        && seed_versions[candidate] < best_version;
    reseeded_flags[candidate] = 0;
    if should_reseed {
        reseeded_flags[candidate] = 1;
    }
    if should_reseed {
        if node_count == 0 {
            current_costs[candidate] = best_cost;
            best_orders_costs[candidate] = best_cost;
        } else {
            let base = candidate * node_count;
            let best_base = best_idx * node_count;
            for idx in 0..node_count {
                let node = best_orders[best_base + idx];
                orders[base + idx] = node;
                best_orders[base + idx] = node;
                positions[base + node as usize] = idx as u32;
            }
            current_costs[candidate] = best_cost;
            best_orders_costs[candidate] = best_cost;
        }
        temp_floors[candidate] = reset_floor;
        stagnant_steps[candidate] = 0;
        seed_versions[candidate] = best_version;
    }
}

#[cube(launch)]
fn minla_sa_copy_kernel(
    node_count: u32,
    old_batch_size: u32,
    orders_old: &Array<u32>,
    positions_old: &Array<u32>,
    best_orders_old: &Array<u32>,
    current_costs_old: &Array<u32>,
    best_orders_costs_old: &Array<u32>,
    temp_floors_old: &Array<f32>,
    rng_states_old: &Array<u32>,
    stagnant_steps_old: &Array<u32>,
    seed_versions_old: &Array<u32>,
    orders_new: &mut Array<u32>,
    positions_new: &mut Array<u32>,
    best_orders_new: &mut Array<u32>,
    current_costs_new: &mut Array<u32>,
    best_orders_costs_new: &mut Array<u32>,
    temp_floors_new: &mut Array<f32>,
    rng_states_new: &mut Array<u32>,
    stagnant_steps_new: &mut Array<u32>,
    seed_versions_new: &mut Array<u32>,
) {
    let candidate = ABSOLUTE_POS;
    let node_count = node_count as usize;
    if candidate < old_batch_size as usize {
        let base = candidate * node_count;
        for idx in 0..node_count {
            let offset = base + idx;
            orders_new[offset] = orders_old[offset];
            positions_new[offset] = positions_old[offset];
            best_orders_new[offset] = best_orders_old[offset];
        }
        current_costs_new[candidate] = current_costs_old[candidate];
        best_orders_costs_new[candidate] = best_orders_costs_old[candidate];
        temp_floors_new[candidate] = temp_floors_old[candidate];
        rng_states_new[candidate] = rng_states_old[candidate];
        stagnant_steps_new[candidate] = stagnant_steps_old[candidate];
        seed_versions_new[candidate] = seed_versions_old[candidate];
    }
}

fn seed_to_u32(seed: u64) -> u32 {
    let low = seed as u32;
    let high = (seed >> 32) as u32;
    low ^ high.wrapping_mul(SEED_MIX)
}

fn cost_cpu(order: &[usize], graph: &GraphData) -> u32 {
    let mut positions = vec![0u32; graph.node_count.max(1)];
    for (pos, &node) in order.iter().take(graph.node_count).enumerate() {
        positions[node] = pos as u32;
    }

    let mut cost = 0u32;
    for &(u, v) in &graph.edges {
        let pu = positions[u];
        let pv = positions[v];
        cost += if pu > pv { pu - pv } else { pv - pu };
    }
    cost
}

#[notebook]
fn main(nb: &mut NotebookCtx) {
    nb.view(|ui| {
        md!(
            ui,
            r#"# MinLA with CubeCL
This notebook explores Minimum Linear Arrangement (MinLA) using CubeCL.

We keep the search deliberately simple:
- The backend generates random permutations from the seed.
- The wgpu backend scores a batch in parallel.
- The notebook keeps the best order seen so far.

A separate card runs a GPU simulated annealing loop for comparison.

The first run can be slow while CubeCL builds shaders.

## SA intuition (quick)
- *Metropolis test*: a move with `delta = new - current` is accepted if `delta <= 0`,
  otherwise with probability `exp(-delta / T)`.
- *Temperature (`T`)* is the scale that decides how often we accept worse moves.
- *Acceptance rate* is just the observed fraction of accepted moves.
- *Temperature floor* stays constant; stagnation is handled by reseeding stale chains."#
        );
    });

    let config = nb.state("config", Config::default(), move |ui, config| {
        ui.with_padding(DEFAULT_CARD_PADDING, |ctx| {
            ctx.label("Batch configuration");
            ctx.checkbox(&mut config.auto_batch, "Adaptive batch size");
            ctx.add_enabled(
                !config.auto_batch,
                widgets::Slider::new(&mut config.batch_size, MIN_BATCH_SIZE..=MAX_BATCH_SIZE)
                    .text("batch"),
            );
            if config.auto_batch {
                ctx.horizontal(|ctx| {
                    ctx.label("target");
                    ctx.add(widgets::Slider::new(&mut config.target_batch_ms, 10..=500).text("ms"));
                });
            }
            ctx.label("backend: wgpu");
            ctx.horizontal(|ctx| {
                ctx.label("seed");
                ctx.add(DragValue::new(&mut config.seed).speed(1));
            });
            ctx.separator();
            ctx.label("Graph");
            ctx.horizontal(|ctx| {
                ctx.label("difficulty");
                ctx.add(widgets::Slider::new(&mut config.difficulty, 0.0..=1.0).text("level"));
            });
            ctx.checkbox(&mut config.use_difficulty, "Use difficulty slider");

            let manual = !config.use_difficulty;
            if manual {
                egui::ComboBox::from_id_salt("minla_graph_pattern")
                    .selected_text(config.graph.pattern.label())
                    .show_ui(ctx, |ctx| {
                        for pattern in GraphPattern::all() {
                            ctx.selectable_value(
                                &mut config.graph.pattern,
                                pattern,
                                pattern.label(),
                            );
                        }
                    });

                let preset = config.graph.pattern == GraphPattern::Preset;
                if preset {
                    config.graph.node_count = DEFAULT_NODE_COUNT;
                    config.graph.edge_count = DEFAULT_EDGE_COUNT;
                }
                ctx.add_enabled(
                    !preset,
                    widgets::Slider::new(&mut config.graph.node_count, 2..=MAX_NODE_COUNT)
                        .text("nodes"),
                );
                let max_edges = max_edges(config.graph.node_count).min(MAX_EDGE_COUNT);
                if config.graph.edge_count > max_edges {
                    config.graph.edge_count = max_edges;
                }
                ctx.add_enabled(
                    !preset,
                    widgets::Slider::new(&mut config.graph.edge_count, 0..=max_edges).text("edges"),
                );
            } else {
                ctx.label("Pattern, nodes, and edges follow the difficulty curve.");
            }

            ctx.horizontal(|ctx| {
                ctx.label("seed");
                ctx.add(DragValue::new(&mut config.graph.seed).speed(1));
                if ctx
                    .add_enabled(
                        !(manual && config.graph.pattern == GraphPattern::Preset),
                        widgets::Button::new("Regenerate"),
                    )
                    .clicked()
                {
                    config.graph.seed = config.graph.seed.wrapping_add(1);
                }
            });
            if manual && config.graph.pattern == GraphPattern::Preset {
                ctx.label("Preset keeps the original 10-node example graph.");
            }

            config.sync_graph();

            if config.use_difficulty {
                ctx.label(format!(
                    "pattern: {}, nodes: {}, edges: {}",
                    config.graph.pattern.label(),
                    config.graph.node_count,
                    config.graph.edge_count
                ));
            }

            let baseline_order: Vec<usize> = (0..config.graph_data.node_count).collect();
            let baseline_cost = cost_cpu(&baseline_order, &config.graph_data);
            widgets::markdown(
                ctx,
                &format!(
                    "Baseline cost (identity order): `{}` for `{}` nodes and `{}` edges.",
                    baseline_cost, config.graph_data.node_count, config.graph_data.edge_count
                ),
            );
        });
    });

    nb.state("minla", MinlaState::new(), move |ui, state| {
        let graph = { config.read(ui).graph_data.clone() };

        state.sync_graph(&graph);
        state.poll_runners();

        let running = state.runner_wgpu.is_running();
        if !running {
            let mut config = config.read_mut(ui);
            if config.auto_batch
                && state.last.error.is_none()
                && state.last.batch_size > 0
                && state.last.elapsed_ms > 0
            {
                let base = state
                    .last
                    .batch_size
                    .min(u32::MAX as usize) as u32;
                config.batch_size = adjust_batch_size(
                    base,
                    state.last.elapsed_ms,
                    config.target_batch_ms,
                );
            }
            config.batch_size = config.batch_size.clamp(MIN_BATCH_SIZE, MAX_BATCH_SIZE);
        }

        let (batch_size, seed) = {
            let config = config.read(ui);
            (config.batch_size.max(1) as usize, config.seed)
        };
        let mut spawn_requested = false;
        let mut bump_seed = false;
        ui.with_padding(DEFAULT_CARD_PADDING, |ctx| {
            ctx.label("Search");
            ctx.horizontal(|ctx| {
                let toggle_label = if state.auto_run {
                    "Auto-run: on"
                } else {
                    "Auto-run: off"
                };
                let auto_run_active = state.auto_run;
                let mut toggle = widgets::Button::new(toggle_label).on(&mut state.auto_run);
                if auto_run_active {
                    let (fill, light) = auto_run_pulse(ctx);
                    toggle = toggle.fill(fill).light(light);
                }
                ctx.add(toggle);
                if !state.auto_run
                    && ctx
                        .add_enabled(!running, widgets::Button::new("Run once"))
                        .clicked()
                {
                    spawn_requested = true;
                }
                if running || state.auto_run {
                    ctx.ctx().request_repaint();
                }
            });
            let last = &state.last;
            if let Some(error) = &last.error {
                widgets::markdown(ctx, &format!("Backend error: {error}"));
            } else if last.batch_size > 0 {
                let throughput = if last.elapsed_ms > 0 {
                    last.batch_size as f64 / last.elapsed_ms as f64
                } else {
                    0.0
                };
                widgets::markdown(
                    ctx,
                    &format!(
                        "Last batch: `{}` candidates in `{}` ms ({:.2} candidates/ms). Best in batch: `{}`.",
                        last.batch_size,
                        last.elapsed_ms,
                        throughput,
                        last.best_cost
                    ),
                );
            } else {
                widgets::markdown(ctx, "No batch yet. Click \"Run once\" to start.");
            }


            if state.best.cost != u32::MAX && !state.best.order.is_empty() {
                widgets::markdown(
                    ctx,
                    &format!(
                        "Best overall: `{}` ({} runs, {} samples).",
                        state.best.cost, state.runs, state.total_samples
                    ),
                );
            }

            if !state.history.is_empty() {
                ctx.separator();
                ctx.label("Performance and improvement");
                ctx.columns(2, |columns| {
                    let perf_points: Vec<[f64; 2]> = state
                        .history
                        .iter()
                        .map(|entry| [entry.run as f64, entry.elapsed_ms as f64])
                        .collect();

                    Plot::new("minla_perf")
                        .height(320.0)
                        .legend(Legend::default())
                        .show(&mut columns[0], |plot_ui| {
                            if !perf_points.is_empty() {
                                plot_ui.line(Line::new("wgpu", PlotPoints::from(perf_points)));
                            }
                        });

                    let best_points: Vec<[f64; 2]> = state
                        .history
                        .iter()
                        .map(|entry| [entry.run as f64, entry.best_cost as f64])
                        .collect();
                    let batch_points: Vec<[f64; 2]> = state
                        .history
                        .iter()
                        .map(|entry| [entry.run as f64, entry.batch_cost as f64])
                        .collect();

                    Plot::new("minla_cost")
                        .height(320.0)
                        .legend(Legend::default())
                        .show(&mut columns[1], |plot_ui| {
                            if !best_points.is_empty() {
                                plot_ui.line(Line::new("best", PlotPoints::from(best_points)));
                            }
                            if !batch_points.is_empty() {
                                plot_ui.line(Line::new("batch", PlotPoints::from(batch_points)));
                            }
                        });
                });
            }
        });

        if !running && state.auto_run {
            spawn_requested = true;
        }
        if spawn_requested && !running {
            bump_seed = true;
            let request = BatchRequest {
                batch_size,
                seed,
                graph: graph.clone(),
            };
            if let Err(error) = state.runner_wgpu.spawn(request) {
                state.last = error;
            }
            ui.ctx().request_repaint();
        }
        if bump_seed {
            let mut config = config.read_mut(ui);
            config.seed = config.seed.wrapping_add(1);
        }
    });

    nb.state("anneal", AnnealState::new(), move |ui, state| {
        let (graph, target_ms) = {
            let config = config.read(ui);
            (config.graph_data.clone(), config.target_batch_ms)
        };

        state.sync_graph(&graph);
        state.poll_runner(target_ms, ui.ctx());
        state.maybe_reset();

        let running = state.runner.is_running();
        let mut spawn_requested = false;
        let mut reset_requested = false;

        ui.with_padding(DEFAULT_CARD_PADDING, |ctx| {
            ctx.label("Simulated annealing (GPU)");
            ctx.label(format!(
                "Auto-tuned parameters (target ~{} ms/batch).",
                target_ms
            ));
            ctx.label(format!(
                "Steps/chain: {}, chains: {} (auto), initial temp: {:.2}.",
                state.config.steps_per_batch,
                state.chain_count,
                state.config.initial_temp
            ));
            ctx.label(format!(
                "Reheat adjust: {:.4}.",
                state.config.cooling_adjust
            ));
            ctx.label(format!(
                "Temp floor: {:.2}, stagnation scale: {} steps.",
                state.config.temp_floor, RESEED_PLATEAU_STEPS
            ));
            ctx.label("Reset re-tunes parameters for the current graph.");

            ctx.horizontal(|ctx| {
                let toggle_label = if state.auto_run {
                    "Auto-run: on"
                } else {
                    "Auto-run: off"
                };
                let auto_run_active = state.auto_run;
                let mut toggle = widgets::Button::new(toggle_label).on(&mut state.auto_run);
                if auto_run_active {
                    let (fill, light) = auto_run_pulse(ctx);
                    toggle = toggle.fill(fill).light(light);
                }
                ctx.add(toggle);
                if !state.auto_run
                    && ctx
                        .add_enabled(!running, widgets::Button::new("Run once"))
                        .clicked()
                {
                    spawn_requested = true;
                }
                if !state.auto_run
                    && ctx
                        .add_enabled(!running, widgets::Button::new("Reset"))
                        .clicked()
                {
                    reset_requested = true;
                }
                if running || state.auto_run {
                    ctx.ctx().request_repaint();
                }
            });

            let last = &state.last;
            if let Some(error) = &last.error {
                widgets::markdown(ctx, &format!("Backend error: {error}"));
            } else if last.batch_size > 0 {
                let chains_per_s = if last.elapsed_ms > 0 {
                    last.batch_size as f64 * 1000.0 / last.elapsed_ms as f64
                } else {
                    0.0
                };
                let steps_per_s = if last.elapsed_ms > 0 {
                    last.batch_size as f64 * last.steps as f64 * 1000.0
                        / last.elapsed_ms as f64
                } else {
                    0.0
                };
                widgets::markdown(
                    ctx,
                    &format!(
                        "Last batch: `{}` chains × `{}` steps in `{}` ms ({:.2} chains/s, {:.0} steps/s). Best: `{}`.",
                        last.batch_size,
                        last.steps,
                        last.elapsed_ms,
                        chains_per_s,
                        steps_per_s,
                        last.best_cost
                    ),
                );
                widgets::markdown(
                    ctx,
                    &format!(
                        "Reseeded chains: `{}` / `{}`.",
                        last.reseeded, last.batch_size
                    ),
                );
            } else {
                widgets::markdown(ctx, "No annealing run yet.");
            }

            if state.best_cost != u32::MAX {
                if state.history.len() >= 2 {
                    let first = &state.history[0];
                    let last_entry = &state.history[state.history.len() - 1];
                    let mut total_ms = 0u128;
                    for entry in &state.history {
                        total_ms += entry.elapsed_ms;
                    }
                    if total_ms > 0 && last_entry.best_cost < first.best_cost {
                        let improvement = (first.best_cost - last_entry.best_cost) as f64;
                        let improvement_per_s = improvement * 1000.0 / total_ms as f64;
                        ctx.label(format!(
                            "Best improvement rate: {:.2} cost/s over last {} runs.",
                            improvement_per_s,
                            state.history.len()
                        ));
                    }
                }
                if let Some(start_cost) = state.start_cost {
                    if state.total_elapsed_ms > 0 {
                        let improvement = start_cost.saturating_sub(state.best_cost) as f64;
                        let improvement_per_s =
                            improvement * 1000.0 / state.total_elapsed_ms as f64;
                        let elapsed_s = state.total_elapsed_ms as f64 / 1000.0;
                        ctx.label(format!(
                            "Overall improvement rate: {:.2} cost/s over {:.1} s.",
                            improvement_per_s, elapsed_s
                        ));
                    }
                }
                widgets::markdown(
                    ctx,
                    &format!(
                        "Best overall: `{}` ({} runs, {} chains, {} steps).",
                        state.best_cost,
                        state.runs,
                        state.total_chains,
                        state.total_steps
                    ),
                );
            }

            if !state.history.is_empty() {
                let best_points: Vec<[f64; 2]> = state
                    .history
                    .iter()
                    .map(|entry| [entry.run as f64, entry.best_cost as f64])
                    .collect();
                let reseed_points: Vec<[f64; 2]> = state
                    .history
                    .iter()
                    .map(|entry| [entry.run as f64, entry.reseeded as f64])
                    .collect();

                Plot::new("anneal_cost")
                    .height(320.0)
                    .legend(Legend::default())
                    .show(ctx, move |plot_ui| {
                        if !best_points.is_empty() {
                            plot_ui.line(Line::new("best", PlotPoints::from(best_points)));
                        }
                    });

                if !reseed_points.is_empty() {
                    Plot::new("anneal_reseeds")
                        .height(120.0)
                        .show(ctx, move |plot_ui| {
                            plot_ui.line(Line::new("reseeds", PlotPoints::from(reseed_points)));
                        });
                }
            }

            ctx.separator();
            ctx.label("Stress over time");
            ctx.horizontal(|ctx| {
                ctx.label("View");
                let prev_view = state.stress_view;
                ctx.add(
                    widgets::ChoiceToggle::new(&mut state.stress_view)
                        .choice(StressView::Tension, "Tension")
                        .choice(StressView::Touched, "Touched"),
                );
                if state.stress_view != prev_view && state.stress_view == StressView::Touched {
                    state.stress.clear_touched();
                }
            });
        });

        state.stress.show(ui, state.stress_view);

        if reset_requested && !running {
            state.reset();
        }
        if !running && state.auto_run {
            spawn_requested = true;
        }
        if spawn_requested && !running {
            let batch_size = state.chain_count.max(1) as usize;
            let steps = state
                .config
                .steps_per_batch
                .clamp(MIN_ANNEAL_STEPS, MAX_ANNEAL_STEPS);
            let plateau_steps = RESEED_PLATEAU_STEPS;
            let seed = state.config.seed;
            let reset = state.force_reinit;
            let request = AnnealRequest {
                batch_size,
                steps,
                seed,
                reset,
                initial_temp: state.config.initial_temp,
                temp_floor: state.config.temp_floor,
                reseed_plateau_steps: plateau_steps,
                cooling_adjust: state.config.cooling_adjust,
                capture_stress: true,
                capture_touched: state.stress_view == StressView::Touched,
                graph: graph.clone(),
            };
            if let Err(error) = state.runner.spawn(request) {
                state.last = error;
            }
            state.force_reinit = false;
            ui.ctx().request_repaint();
        }
    });
}
