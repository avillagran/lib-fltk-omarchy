//! Managed position/opacity transitions for retained keyed layers.
use super::{NodeError, NodeId, Scene, Surface};
use crate::anim::{self, Ease, Tween};
use std::collections::HashMap;

pub(super) type Done = Box<dyn FnOnce(&Surface, NodeId)>;

/// A one-shot keyed transition, driven by the shared toolkit clock.
/// Defaults to the toolkit's 190 ms OutCubic layout transition. Omitted
/// properties stay unchanged. Reduced motion and zero duration settle
/// immediately; callbacks run on a later UI turn, outside drawing.
pub struct NodeMotion {
    position: Option<(i32, i32)>,
    opacity: Option<f32>,
    ms: u32,
    ease: Ease,
    done: Option<Done>,
}

impl Default for NodeMotion {
    fn default() -> Self {
        Self {
            position: None,
            opacity: None,
            ms: anim::ms::LAYOUT,
            ease: Ease::OutCubic,
            done: None,
        }
    }
}
impl NodeMotion {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn position(mut self, x: i32, y: i32) -> Self {
        self.position = Some((x, y));
        self
    }
    pub fn opacity(mut self, value: f32) -> Self {
        self.opacity = Some(value);
        self
    }
    pub fn duration_ms(mut self, value: u32) -> Self {
        self.ms = value;
        self
    }
    pub fn ease(mut self, value: Ease) -> Self {
        self.ease = value;
        self
    }
    /// Receives the live surface and completed ID. Retargeting, an
    /// immediate setter, cancellation or replacing content drops the old
    /// callback, including one queued but not delivered yet.
    pub fn on_done(mut self, callback: impl FnOnce(&Surface, NodeId) + 'static) -> Self {
        self.done = Some(Box::new(callback));
        self
    }
}

#[derive(Clone, Copy)]
pub(super) struct Frame {
    pub id: NodeId,
    pub position: Option<(i32, i32)>,
    pub opacity: Option<f32>,
}
impl Frame {
    fn apply(self, scene: &mut Scene) -> Result<(), NodeError> {
        scene.node_bounds(self.id)?;
        if let Some((x, y)) = self.position {
            scene.move_node(self.id, x, y)?;
        }
        if let Some(alpha) = self.opacity {
            scene.set_node_opacity(self.id, alpha)?;
        }
        Ok(())
    }
}
struct Active {
    from: (i32, i32, f32),
    target: NodeMotion,
    clock: Tween,
    serial: u64,
}
impl Active {
    fn sample(&self, id: NodeId, settle: bool) -> Frame {
        let t = if settle || self.clock.done() {
            1.0
        } else {
            self.clock.value()
        };
        let position = self.target.position.map(|(x, y)| {
            let lerp =
                |from: i32, to: i32| (from as f64 + (to as f64 - from as f64) * t).round() as i32;
            (lerp(self.from.0, x), lerp(self.from.1, y))
        });
        let opacity = self.target.opacity.map(|to| {
            (self.from.2 as f64 + (to as f64 - self.from.2 as f64) * t).clamp(0.0, 1.0) as f32
        });
        Frame {
            id,
            position,
            opacity,
        }
    }
}

#[derive(Default)]
pub(super) struct Motions {
    running: HashMap<NodeId, Active>,
    serials: HashMap<NodeId, u64>,
    next: u64,
    pub frame: Vec<Frame>,
    pub done: Vec<(NodeId, u64, Done)>,
}
impl Motions {
    pub fn active(&self, id: NodeId) -> bool {
        self.running.contains_key(&id)
    }
    pub fn any(&self) -> bool {
        !self.running.is_empty()
    }
    pub fn current(&self, id: NodeId, serial: u64) -> bool {
        self.serials.get(&id) == Some(&serial)
    }
    fn serial(&mut self, id: NodeId) -> u64 {
        self.next = self.next.wrapping_add(1).max(1);
        self.serials.insert(id, self.next);
        self.next
    }
    pub fn clear(&mut self) {
        self.running.clear();
        self.serials.clear();
        self.frame.clear();
        self.done.clear();
    }
    pub fn start(
        &mut self,
        scene: &mut Scene,
        id: NodeId,
        mut target: NodeMotion,
        hidden: bool,
    ) -> Result<(), NodeError> {
        // Validation precedes sampling/cancelling an existing transition.
        scene.node_bounds(id)?;
        if let Some(alpha) = target.opacity {
            target.opacity = Some(super::node::opacity(alpha)? as f32 / 255.0);
        }
        if matches!(target.ease, Ease::OutBack(s) if !s.is_finite()) {
            return Err(NodeError::InvalidEasing);
        }
        self.sample_one(scene, id, !anim::enabled());
        self.running.remove(&id);
        let serial = self.serial(id);
        let r = scene.node_bounds(id)?;
        let from = (r.x, r.y, scene.node_opacity(id)?);
        let unchanged = target.position.map_or(true, |p| p == (from.0, from.1))
            && target.opacity.map_or(true, |alpha| alpha == from.2);
        let immediate = !anim::enabled() || hidden || target.ms == 0 || unchanged;
        let clock = Tween::start(target.ms, target.ease);
        let mut active = Active {
            from,
            target,
            clock,
            serial,
        };
        if immediate {
            let frame = active.sample(id, true);
            frame.apply(scene)?;
            self.frame.push(frame);
            if let Some(done) = active.target.done.take() {
                self.done.push((id, serial, done));
            }
        } else {
            self.running.insert(id, active);
        }
        Ok(())
    }
    /// Samples before cancelling so a drag/retarget starts at the pose
    /// visible at the current clock time, including unpainted elapsed time.
    pub fn cancel(&mut self, scene: &mut Scene, id: NodeId) -> Result<(), NodeError> {
        scene.node_bounds(id)?;
        self.sample_one(scene, id, !anim::enabled());
        self.running.remove(&id);
        if self.serials.contains_key(&id) {
            self.serial(id);
        }
        Ok(())
    }
    fn sample_one(&mut self, scene: &mut Scene, id: NodeId, settle: bool) {
        self.frame.clear();
        if let Some(active) = self.running.get(&id) {
            let frame = active.sample(id, settle);
            if frame.apply(scene).is_ok() {
                self.frame.push(frame);
            }
        }
    }
    pub fn sample(&mut self, scene: &mut Scene, settle: bool) {
        self.frame.clear();
        let Self {
            running,
            frame,
            done,
            serials,
            ..
        } = self;
        running.retain(|id, active| {
            let update = active.sample(*id, settle);
            if update.apply(scene).is_err() {
                serials.remove(id);
                return false;
            }
            frame.push(update);
            if settle || active.clock.done() {
                if let Some(callback) = active.target.done.take() {
                    done.push((*id, active.serial, callback));
                }
                false
            } else {
                true
            }
        });
    }
    pub fn prune(&mut self, scene: &Scene) {
        self.serials.retain(|id, _| scene.node_bounds(*id).is_ok());
        self.running.retain(|id, _| self.serials.contains_key(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Layer, Rect};
    use std::time::Duration;
    struct Clock;
    impl Clock {
        fn new() -> Self {
            anim::set_enabled(true);
            anim::clock::freeze();
            Self
        }
    }
    impl Drop for Clock {
        fn drop(&mut self) {
            anim::clock::resume();
            anim::set_enabled(None);
        }
    }
    fn fixture() -> Scene {
        let layer = Layer::new(Rect::new(0, 0, 20, 20), Scene::default()).unwrap();
        Scene::record(|| {
            layer.draw_keyed(NodeId(1), 0, 0, 1.0).unwrap();
        })
        .0
    }
    #[test]
    fn invalid_retarget_is_atomic_and_cancellation_holds_the_current_pose() {
        let _clock = Clock::new();
        let mut scene = fixture();
        let mut motion = Motions::default();
        motion
            .start(
                &mut scene,
                NodeId(1),
                NodeMotion::new()
                    .position(100, 40)
                    .duration_ms(200)
                    .ease(Ease::Linear),
                false,
            )
            .unwrap();
        anim::clock::advance(Duration::from_millis(100));
        assert_eq!(
            motion.start(
                &mut scene,
                NodeId(1),
                NodeMotion::new().opacity(f32::NAN),
                false
            ),
            Err(NodeError::InvalidOpacity)
        );
        assert_eq!(
            motion.start(
                &mut scene,
                NodeId(1),
                NodeMotion::new().ease(Ease::OutBack(f64::INFINITY)),
                false
            ),
            Err(NodeError::InvalidEasing)
        );
        assert!(motion.active(NodeId(1)));
        assert_eq!(scene.node_bounds(NodeId(1)).unwrap().x, 0);
        motion.cancel(&mut scene, NodeId(1)).unwrap();
        assert_eq!(
            scene.node_bounds(NodeId(1)).unwrap(),
            Rect::new(50, 20, 20, 20)
        );
        assert!(!motion.any());
        anim::clock::advance(Duration::from_secs(1));
        motion.sample(&mut scene, false);
        assert_eq!(scene.node_bounds(NodeId(1)).unwrap().x, 50);
    }
    #[test]
    fn unchanged_hidden_and_reduced_motion_targets_do_not_start_a_frame_loop() {
        let _clock = Clock::new();
        let mut scene = fixture();
        let mut motion = Motions::default();
        motion
            .start(
                &mut scene,
                NodeId(1),
                NodeMotion::new().position(0, 0).opacity(1.0),
                false,
            )
            .unwrap();
        assert!(!motion.any());
        motion
            .start(
                &mut scene,
                NodeId(1),
                NodeMotion::new().position(100, 20),
                true,
            )
            .unwrap();
        assert!(!motion.any());
        assert_eq!(scene.node_bounds(NodeId(1)).unwrap().x, 100);
        anim::set_enabled(false);
        motion
            .start(
                &mut scene,
                NodeId(1),
                NodeMotion::new().position(200, 20),
                false,
            )
            .unwrap();
        assert!(!motion.any());
        assert_eq!(scene.node_bounds(NodeId(1)).unwrap().x, 200);
    }
}
