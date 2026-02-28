use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::time::{interval, Duration};
use crate::storage::backend::Storage;
use metrics::{Recorder, Key, KeyName, Unit, SharedString, Counter, Gauge, Histogram, Metadata};
use tracing::{Id, Subscriber};
use tracing_subscriber::{Layer, registry::LookupSpan};
use serde::Serialize;

use metrics::{CounterFn, GaugeFn, HistogramFn};

// --- Metrics Recorder ---

struct VardaRecorder {
    #[allow(dead_code)]
    storage: Arc<Storage>, // Kept for potential direct writes in future, though mostly used by flusher
    counters: Arc<dashmap::DashMap<String, Arc<AtomicU64>>>,
    gauges: Arc<dashmap::DashMap<String, Arc<AtomicU64>>>, // Store as f64 bits
    histograms: Arc<dashmap::DashMap<String, Arc<RwLock<Vec<f64>>>>>,
}

impl VardaRecorder {
    fn new(storage: Arc<Storage>) -> Self {
        Self {
            storage,
            counters: Arc::new(dashmap::DashMap::new()),
            gauges: Arc::new(dashmap::DashMap::new()),
            histograms: Arc::new(dashmap::DashMap::new()),
        }
    }

    fn key_to_string(key: &Key) -> String {
        let name = key.name().to_string();
        let labels = key.labels();
        // Consuming iterator is fine here
        let mut label_parts: Vec<String> = labels.map(|l| format!("{}={}", l.key(), l.value())).collect();
        if label_parts.is_empty() {
            name
        } else {
            label_parts.sort();
            format!("{}{{{}}}", name, label_parts.join(","))
        }
    }
}

struct VardaCounter {
    val: Arc<AtomicU64>,
}
impl CounterFn for VardaCounter {
    fn increment(&self, value: u64) {
        self.val.fetch_add(value, Ordering::Relaxed);
    }
    fn absolute(&self, value: u64) {
        self.val.store(value, Ordering::Relaxed);
    }
}

struct VardaGauge {
    val: Arc<AtomicU64>, // f64 bits
}
impl GaugeFn for VardaGauge {
    fn increment(&self, value: f64) {
        let mut current = self.val.load(Ordering::Relaxed);
        loop {
            let current_f = f64::from_bits(current);
            let new_f = current_f + value;
            let new_bits = new_f.to_bits();
            match self.val.compare_exchange_weak(current, new_bits, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(x) => current = x,
            }
        }
    }
    fn decrement(&self, value: f64) {
        self.increment(-value);
    }
    fn set(&self, value: f64) {
        self.val.store(value.to_bits(), Ordering::Relaxed);
    }
}

struct VardaHistogram {
    vals: Arc<RwLock<Vec<f64>>>,
}
impl HistogramFn for VardaHistogram {
    fn record(&self, value: f64) {
        self.vals.write().push(value);
    }
}


impl Recorder for VardaRecorder {
    fn describe_counter(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}
    fn describe_gauge(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}
    fn describe_histogram(&self, _key: KeyName, _unit: Option<Unit>, _description: SharedString) {}

    fn register_counter(&self, key: &Key, _metadata: &Metadata<'_>) -> Counter {
        let key_str = Self::key_to_string(key);
        // or_insert returns RefMut. We access value() which is Arc<AtomicU64>. clone() increments ref count.
        let val = self.counters.entry(key_str).or_insert(Arc::new(AtomicU64::new(0))).value().clone();
        Counter::from_arc(Arc::new(VardaCounter { val }))
    }

    fn register_gauge(&self, key: &Key, _metadata: &Metadata<'_>) -> Gauge {
         let key_str = Self::key_to_string(key);
         let val = self.gauges.entry(key_str).or_insert(Arc::new(AtomicU64::new(0))).value().clone();
         Gauge::from_arc(Arc::new(VardaGauge { val }))
    }

    fn register_histogram(&self, key: &Key, _metadata: &Metadata<'_>) -> Histogram {
         let key_str = Self::key_to_string(key);
         let val = self.histograms.entry(key_str).or_insert(Arc::new(RwLock::new(Vec::new()))).value().clone();
         Histogram::from_arc(Arc::new(VardaHistogram { vals: val }))
    }
}

// --- Trace Layer ---

pub struct VardaTraceLayer {
    storage: Arc<Storage>,
}

impl VardaTraceLayer {
    pub fn new(storage: Arc<Storage>) -> Self {
        Self { storage }
    }
}

#[derive(Serialize)]
struct TraceSpan {
    name: String,
    start_ts: u64,
    duration_ms: u64,
    fields: HashMap<String, String>,
    children: Vec<TraceSpan>,
}

#[derive(Default)]
struct SpanData {
    name: String,
    start_ts: u64,
    fields: HashMap<String, String>,
}

impl<S> Layer<S> for VardaTraceLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &tracing::span::Attributes<'_>, id: &Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut data = SpanData::default();
            data.name = attrs.metadata().name().to_string();
            data.start_ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
            
            // Visitor to capture fields
            struct FieldVisitor<'a>(&'a mut HashMap<String, String>);
            impl<'a> tracing::field::Visit for FieldVisitor<'a> {
                fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                    self.0.insert(field.name().to_string(), format!("{:?}", value));
                }
            }
            attrs.record(&mut FieldVisitor(&mut data.fields));
            
            span.extensions_mut().insert(data);
        }
    }

    fn on_record(&self, id: &Id, values: &tracing::span::Record<'_>, ctx: tracing_subscriber::layer::Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut extensions = span.extensions_mut();
            if let Some(data) = extensions.get_mut::<SpanData>() {
                 struct FieldVisitor<'a>(&'a mut HashMap<String, String>);
                impl<'a> tracing::field::Visit for FieldVisitor<'a> {
                    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                        self.0.insert(field.name().to_string(), format!("{:?}", value));
                    }
                }
                values.record(&mut FieldVisitor(&mut data.fields));
            }
        }
    }

    fn on_enter(&self, _id: &Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {}
    fn on_exit(&self, _id: &Id, _ctx: tracing_subscriber::layer::Context<'_, S>) {}

    fn on_close(&self, id: Id, ctx: tracing_subscriber::layer::Context<'_, S>) {
        if let Some(span) = ctx.span(&id) {
            if span.parent().is_none() {
                let extensions = span.extensions();
                if let Some(data) = extensions.get::<SpanData>() {
                     let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
                     let duration = now - data.start_ts;
                     
                     let trace = TraceSpan {
                         name: data.name.clone(),
                         start_ts: data.start_ts,
                         duration_ms: duration,
                         fields: data.fields.clone(),
                         children: vec![], 
                     };
                     
                     if let Ok(json) = serde_json::to_vec(&trace) {
                         let mut key = Vec::with_capacity(16);
                         key.extend_from_slice(&data.start_ts.to_be_bytes());
                         key.extend_from_slice(&(duration as u32).to_be_bytes()); 
                         
                         let _ = self.storage.traces_table.insert(&key, &json);
                     }
                }
            }
        }
    }
}

// --- Init ---

pub fn init(storage: Arc<Storage>) {
    let recorder = VardaRecorder::new(storage.clone());
    let recorder_arc = Arc::new(recorder);
    
    let flusher_counters = recorder_arc.counters.clone();
    let flusher_gauges = recorder_arc.gauges.clone();
    let flusher_histograms = recorder_arc.histograms.clone();
    let flusher_storage = storage.clone();
    
    metrics::set_global_recorder(recorder_arc).expect("Failed to set metrics recorder");

    println!("observability: Metrics Recorder initialized.");

    tokio::spawn(async move {
        // Move System init OUTSIDE the loop
        use sysinfo::{System, RefreshKind, CpuRefreshKind, MemoryRefreshKind};
        
        let mut sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything())
        );
            
        let mut interval = interval(Duration::from_secs(10));
        
        // Sleep once to allow CPU usage calculation (needs diff)
        sys.refresh_cpu_all();
        tokio::time::sleep(Duration::from_millis(200)).await;
        sys.refresh_cpu_all();

        loop {
            interval.tick().await;
            let now_ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
            
            // Counters
            for entry in flusher_counters.iter() {
                let key_str = entry.key();
                let val = entry.value().load(Ordering::Relaxed);
                let db_key = format!("c:{}:{}", key_str, now_ts);
                let _ = flusher_storage.metrics_table.insert(db_key.as_bytes(), &val.to_be_bytes());
            }
            
            // Gauges
             for entry in flusher_gauges.iter() {
                let key_str = entry.key();
                let val_bits = entry.value().load(Ordering::Relaxed);
                let db_key = format!("g:{}:{}", key_str, now_ts);
                let _ = flusher_storage.metrics_table.insert(db_key.as_bytes(), &val_bits.to_be_bytes());
            }
            
            // Histograms
             for entry in flusher_histograms.iter() {
                 let key_str = entry.key();
                 let mut lock = entry.value().write();
                 if lock.is_empty() { continue; }
                 
                 lock.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
                 let len = lock.len();
                 let p50 = lock[len / 2];
                 let p90 = lock[(len as f64 * 0.9) as usize];
                 lock.clear();
                 
                 let db_key_p50 = format!("h:{}:p50:{}", key_str, now_ts);
                 let _ = flusher_storage.metrics_table.insert(db_key_p50.as_bytes(), &p50.to_be_bytes());
                 
                 let db_key_p90 = format!("h:{}:p90:{}", key_str, now_ts);
                 let _ = flusher_storage.metrics_table.insert(db_key_p90.as_bytes(), &p90.to_be_bytes());
             }
             
             // System Metrics
             sys.refresh_cpu_all();
             sys.refresh_memory();
             
             let cpu_usage = sys.global_cpu_usage();
             let mem_usage = sys.used_memory();
             
             let k_cpu = format!("g:system.cpu:{}", now_ts);
             let _ = flusher_storage.metrics_table.insert(k_cpu.as_bytes(), &(cpu_usage as f64).to_be_bytes());
             
             let k_mem = format!("g:system.memory:{}", now_ts);
             let _ = flusher_storage.metrics_table.insert(k_mem.as_bytes(), &(mem_usage as f64).to_be_bytes());
        }
    }); 
}
