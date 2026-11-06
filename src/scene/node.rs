//! Stable identities for retained isolated layers.
use super::{Command, Rect, Scene};
use std::{collections::HashMap, fmt};

/// Application-owned identity, unique among the top-level keyed layers in
/// one scene. It need not match paint order or visible primitive indexes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeError {
    NotFound(NodeId),
    Duplicate(NodeId),
    InvalidOpacity,
}
impl fmt::Display for NodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "scene node {} not found", id.0),
            Self::Duplicate(id) => write!(f, "duplicate scene node {}", id.0),
            Self::InvalidOpacity => f.write_str("scene node opacity must be finite"),
        }
    }
}
impl std::error::Error for NodeError {}

pub(super) type Index = Result<HashMap<NodeId, usize>, NodeError>;

pub(super) fn opacity(value: f32) -> Result<u8, NodeError> {
    if !value.is_finite() {
        return Err(NodeError::InvalidOpacity);
    }
    Ok((value.clamp(0.0, 1.0) * 255.0).round() as u8)
}

impl Scene {
    pub(super) fn validate_nodes(&self) -> Result<(), NodeError> {
        // Constructed once per recorded/structurally changed scene. A
        // duplicate invalidates addressing before any mutation happens.
        let mut cached = self.nodes.borrow_mut();
        if cached.is_none() {
            let mut index = HashMap::new();
            let result = self.commands.iter().enumerate().try_for_each(|(i, c)| {
                if let Command::Layer { id: Some(id), .. } = c {
                    if index.insert(*id, i).is_some() {
                        return Err(NodeError::Duplicate(*id));
                    }
                }
                Ok(())
            });
            *cached = Some(result.map(|_| index));
        }
        cached
            .as_ref()
            .unwrap()
            .as_ref()
            .map(|_| ())
            .map_err(Clone::clone)
    }

    fn node_command(&self, id: NodeId) -> Result<usize, NodeError> {
        self.validate_nodes()?;
        self.nodes
            .borrow()
            .as_ref()
            .unwrap()
            .as_ref()
            .map_err(Clone::clone)?
            .get(&id)
            .copied()
            .ok_or(NodeError::NotFound(id))
    }

    /// Current local bounds, including position changes. Ancestor clips
    /// remain fixed; this does not traverse inside isolated child layers.
    pub fn node_bounds(&self, id: NodeId) -> Result<Rect, NodeError> {
        let i = self.node_command(id)?;
        Ok(self.commands[i].bounds().unwrap())
    }

    /// Moves one keyed isolated layer without rebuilding its raster.
    /// Its size, paint order and ancestor clip are unchanged. Invalidates
    /// only the optional whole-scene damage raster, not layer resources.
    pub fn move_node(&mut self, id: NodeId, x: i32, y: i32) -> Result<(), NodeError> {
        let i = self.node_command(id)?;
        let Command::Layer { rect, .. } = &mut self.commands[i] else {
            unreachable!()
        };
        rect.x = x;
        rect.y = y;
        *self.raster.get_mut() = None;
        Ok(())
    }

    /// Sets group opacity once, quantized to the same 8-bit value on CPU
    /// and GL. Clamps finite values to 0..=1; rejects NaN and infinity.
    pub fn set_node_opacity(&mut self, id: NodeId, value: f32) -> Result<(), NodeError> {
        let value = opacity(value)?;
        let i = self.node_command(id)?;
        let Command::Layer { opacity, .. } = &mut self.commands[i] else {
            unreachable!()
        };
        *opacity = value;
        *self.raster.get_mut() = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Layer;

    fn layer() -> Layer {
        Layer::new(Rect::new(0, 0, 20, 15), Scene::default()).unwrap()
    }

    #[test]
    fn stable_ids_survive_clips_offscreen_positions_zero_opacity_and_append() {
        let layer = layer();
        let (mut scene, ()) = Scene::record(|| {
            super::super::clip(Rect::new(5, 5, 60, 60), || {
                layer.draw_keyed(NodeId(91), -100, -100, 0.0).unwrap();
            });
            layer.draw_keyed(NodeId(12), 10, 10, 1.0).unwrap();
        });
        scene.move_node(NodeId(91), 40, 50).unwrap();
        scene.set_node_opacity(NodeId(91), 0.5).unwrap();
        assert_eq!(scene.node_bounds(NodeId(91)), Ok(Rect::new(40, 50, 20, 15)));
        assert!(matches!(
            scene.commands[1],
            Command::Layer { opacity: 128, .. }
        ));
        let (tail, ()) = Scene::record(|| {
            layer.draw_keyed(NodeId(27), 7, 8, 1.0).unwrap();
        });
        scene.append(&tail);
        scene.move_node(NodeId(27), 2, 3).unwrap();
        assert_eq!(scene.node_bounds(NodeId(27)), Ok(Rect::new(2, 3, 20, 15)));
        assert_eq!(scene.node_bounds(NodeId(91)), Ok(Rect::new(40, 50, 20, 15)));
    }

    #[test]
    fn duplicate_ids_fail_before_mutation_and_invalid_values_leave_content_unchanged() {
        let layer = layer();
        let (mut scene, ()) = Scene::record(|| {
            layer.draw_keyed(NodeId(1), 7, 8, 1.0).unwrap();
        });
        assert_eq!(
            scene.move_node(NodeId(2), 0, 0),
            Err(NodeError::NotFound(NodeId(2)))
        );
        for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(
                scene.set_node_opacity(NodeId(1), value),
                Err(NodeError::InvalidOpacity)
            );
        }
        assert_eq!(scene.node_bounds(NodeId(1)), Ok(Rect::new(7, 8, 20, 15)));
        let copy = scene.clone();
        scene.append(&copy);
        assert_eq!(
            scene.move_node(NodeId(1), 0, 0),
            Err(NodeError::Duplicate(NodeId(1)))
        );
        assert!(matches!(
            scene.commands[0],
            Command::Layer {
                rect: Rect { x: 7, y: 8, .. },
                opacity: 255,
                ..
            }
        ));
        #[cfg(feature = "gpu")]
        assert!(
            super::super::gpu::PreparedScene::new(&scene, Rect::new(0, 0, 20, 20))
                .err()
                .unwrap()
                .contains("duplicate")
        );
    }
}
