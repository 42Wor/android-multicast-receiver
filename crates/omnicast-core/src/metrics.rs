//! Thread-safe stream telemetry for FPS, throughput, and session uptime.

use crate::session::DeviceId;
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(1);

/// Point-in-time view of stream health for HUD / settings consumers.
#[derive(Clone, Debug, Default)]
pub struct MetricsSnapshot {
    pub fps: f32,
    pub bitrate_bps: f64,
    pub uptime: Duration,
    pub frames: u64,
    pub packets: u64,
    pub drops: u64,
}

impl MetricsSnapshot {
    pub fn bitrate_label(&self) -> String {
        let mbps = self.bitrate_bps / 1_000_000.0;
        if mbps >= 1.0 {
            format!("{mbps:.1} Mbps")
        } else {
            format!("{:.1} Kbps", self.bitrate_bps / 1_000.0)
        }
    }

    pub fn fps_label(&self) -> String {
        format!("{:.1} FPS", self.fps)
    }

    pub fn uptime_label(&self) -> String {
        let total = self.uptime.as_secs();
        let h = total / 3600;
        let m = (total % 3600) / 60;
        let s = total % 60;
        format!("{h:02}:{m:02}:{s:02}")
    }
}

#[derive(Debug)]
struct MetricsInner {
    connected_at: Instant,
    frame_marks: VecDeque<Instant>,
    byte_marks: VecDeque<(Instant, u64)>,
    frames: u64,
    packets: u64,
    drops: u64,
}

impl MetricsInner {
    fn new() -> Self {
        Self {
            connected_at: Instant::now(),
            frame_marks: VecDeque::new(),
            byte_marks: VecDeque::new(),
            frames: 0,
            packets: 0,
            drops: 0,
        }
    }

    fn prune(&mut self, now: Instant) {
        while let Some(front) = self.frame_marks.front() {
            if now.duration_since(*front) > WINDOW {
                self.frame_marks.pop_front();
            } else {
                break;
            }
        }
        while let Some((t, _)) = self.byte_marks.front() {
            if now.duration_since(*t) > WINDOW {
                self.byte_marks.pop_front();
            } else {
                break;
            }
        }
    }

    fn snapshot(&mut self) -> MetricsSnapshot {
        let now = Instant::now();
        self.prune(now);
        let bytes: u64 = self.byte_marks.iter().map(|(_, b)| *b).sum();
        let elapsed = WINDOW.as_secs_f64().max(0.001);
        MetricsSnapshot {
            fps: self.frame_marks.len() as f32,
            bitrate_bps: (bytes as f64 * 8.0) / elapsed,
            uptime: now.duration_since(self.connected_at),
            frames: self.frames,
            packets: self.packets,
            drops: self.drops,
        }
    }
}

/// Per-stream metrics handle — cheap to clone (`Arc`), safe across threads.
#[derive(Clone, Debug)]
pub struct StreamMetrics {
    inner: Arc<Mutex<MetricsInner>>,
}

impl Default for StreamMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamMetrics {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MetricsInner::new())),
        }
    }

    pub fn record_frame(&self) {
        let mut g = self.inner.lock();
        let now = Instant::now();
        g.frames += 1;
        g.frame_marks.push_back(now);
        g.prune(now);
    }

    pub fn record_network(&self, bytes: u64, packets: u64) {
        let mut g = self.inner.lock();
        let now = Instant::now();
        g.packets += packets;
        g.byte_marks.push_back((now, bytes));
        g.prune(now);
    }

    pub fn record_drop(&self, count: u64) {
        let mut g = self.inner.lock();
        g.drops = g.drops.saturating_add(count);
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        self.inner.lock().snapshot()
    }
}

/// Shared registry so protocol / media threads can update the active session.
#[derive(Clone, Default)]
pub struct MetricsRegistry {
    map: Arc<Mutex<HashMap<DeviceId, StreamMetrics>>>,
    active: Arc<Mutex<Option<DeviceId>>>,
}

impl MetricsRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, id: DeviceId, metrics: StreamMetrics) {
        let mut map = self.map.lock();
        map.insert(id, metrics);
        *self.active.lock() = Some(id);
    }

    pub fn unregister(&self, id: DeviceId) {
        self.map.lock().remove(&id);
        let mut active = self.active.lock();
        if *active == Some(id) {
            *active = self.map.lock().keys().next().copied();
        }
    }

    pub fn get(&self, id: DeviceId) -> Option<StreamMetrics> {
        self.map.lock().get(&id).cloned()
    }

    pub fn set_active(&self, id: DeviceId) {
        *self.active.lock() = Some(id);
    }

    pub fn record_network_active(&self, bytes: u64, packets: u64) {
        if let Some(id) = *self.active.lock() {
            if let Some(m) = self.get(id) {
                m.record_network(bytes, packets);
            }
        }
    }

    pub fn record_drop_active(&self, count: u64) {
        if let Some(id) = *self.active.lock() {
            if let Some(m) = self.get(id) {
                m.record_drop(count);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn rolling_fps_and_bitrate() {
        let m = StreamMetrics::new();
        for _ in 0..30 {
            m.record_frame();
            m.record_network(1_000, 1);
        }
        thread::sleep(Duration::from_millis(20));
        let snap = m.snapshot();
        assert!(snap.frames >= 30);
        assert!(snap.fps >= 1.0);
        assert!(snap.bitrate_bps > 0.0);
        assert!(!snap.uptime_label().is_empty());
    }
}
