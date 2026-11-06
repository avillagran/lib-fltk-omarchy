//! Stable identities for retained isolated layers.
use super::{Command, Rect, Scene};
#[cfg(feature = "gpu")]
use std::ops::Range;
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
    InvalidEasing,
}
impl fmt::Display for NodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "scene node {} not found", id.0),
            Self::Duplicate(id) => write!(f, "duplicate scene node {}", id.0),
            Self::InvalidOpacity => f.write_str("scene node opacity must be finite"),
            Self::InvalidEasing => f.write_str("scene node easing must be finite"),
        }
    }
}
impl std::error::Error for NodeError {}

pub(super) type Index = Result<HashMap<NodeId, usize>, NodeError>;

// Original command coordinates are stable across prepared-draw reordering.
// Clip scopes remain contiguous: a node can only rise within its own scope.
#[cfg(feature = "gpu")]
#[derive(Clone)]
pub(super) struct Order {
    pub source: usize,
    pub scope: Range<usize>,
}
#[cfg(feature = "gpu")]
pub(super) fn scope_ends(commands: &[Command]) -> Vec<usize> {
    let mut ends = vec![commands.len(); commands.len()];
    let mut stack = Vec::new();
    for (i, command) in commands.iter().enumerate() {
        match command {
            Command::PushClip(_) => stack.push(i),
            Command::PopClip => {
                if let Some(start) = stack.pop() {
                    ends[start] = i;
                }
            }
            _ => {}
        }
    }
    ends
}

pub(super) fn raised_indices(nodes: &mut HashMap<NodeId, usize>, from: usize, to: usize) {
    for i in nodes.values_mut() {
        if *i == from {
            *i = to;
        } else if *i > from && *i <= to {
            *i -= 1;
        }
    }
}

#[cfg(feature = "gpu")]
pub(super) fn raise<T>(
    items: &mut [T],
    nodes: &mut HashMap<NodeId, usize>,
    id: NodeId,
    order: impl Fn(&T) -> &Order,
) -> Result<(), NodeError> {
    let from = *nodes.get(&id).ok_or(NodeError::NotFound(id))?;
    let scope = &order(&items[from]).scope;
    let to = items
        .iter()
        .rposition(|item| scope.contains(&order(item).source))
        .unwrap();
    items[from..=to].rotate_left(1);
    raised_indices(nodes, from, to);
    Ok(())
}

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
    pub fn node_opacity(&self, id: NodeId) -> Result<f32, NodeError> {
        let i = self.node_command(id)?;
        let Command::Layer { opacity, .. } = self.commands[i] else {
            unreachable!()
        };
        Ok(opacity as f32 / 255.0)
    }

    /// Raises a keyed layer to the end of its immediate clip scope. It
    /// stays inside every original ancestor clip and below later content
    /// outside that scope. Root nodes rise to the end of the scene.
    /// O(commands + nodes), without rasterization or resource allocation.
    pub fn raise_node(&mut self, id: NodeId) -> Result<(), NodeError> {
        let from = self.node_command(id)?;
        let mut depth = 0usize;
        let mut end = self.commands.len();
        for (i, command) in self.commands.iter().enumerate().skip(from + 1) {
            match command {
                Command::PushClip(_) => depth += 1,
                Command::PopClip if depth == 0 => {
                    end = i;
                    break;
                }
                Command::PopClip => depth -= 1,
                _ => {}
            }
        }
        self.commands[from..end].rotate_left(1);
        raised_indices(
            self.nodes.get_mut().as_mut().unwrap().as_mut().unwrap(),
            from,
            end - 1,
        );
        *self.raster.get_mut() = None;
        Ok(())
    }

    /// Last painted keyed layer whose rectangular bounds contain the
    /// point, respecting ancestor clips and zero group opacity. Unkeyed
    /// visual commands do not consume hits. This is semantic rectangular
    /// hit testing, not a texture-alpha read-back or a nested-node query.
    pub fn hit_node(&self, x: i32, y: i32) -> Result<Option<NodeId>, NodeError> {
        self.validate_nodes()?;
        let mut depth = 0usize;
        let mut clipped = None;
        let mut hit = None;
        for command in &self.commands {
            match command {
                Command::PushClip(rect) => {
                    depth += 1;
                    if clipped.is_none() && !rect.contains(x, y) {
                        clipped = Some(depth);
                    }
                }
                Command::PopClip => {
                    if clipped == Some(depth) {
                        clipped = None;
                    }
                    depth = depth.saturating_sub(1);
                }
                Command::Layer {
                    rect,
                    id: Some(id),
                    opacity,
                    ..
                } if *opacity != 0 && clipped.is_none() && rect.contains(x, y) => hit = Some(*id),
                _ => {}
            }
        }
        Ok(hit)
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
        assert_eq!(
            scene.raise_node(NodeId(1)),
            Err(NodeError::Duplicate(NodeId(1)))
        );
        assert_eq!(scene.hit_node(8, 9), Err(NodeError::Duplicate(NodeId(1))));
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

    #[test]
    fn raised_nodes_and_hits_keep_ancestor_clips_and_skip_zero_alpha() {
        let layer = layer();
        let (mut scene, ()) = Scene::record(|| {
            super::super::clip(Rect::new(0, 0, 12, 12), || {
                layer.draw_keyed(NodeId(1), 0, 0, 1.0).unwrap();
                super::super::clip(Rect::new(0, 0, 6, 6), || {
                    layer.draw_keyed(NodeId(2), 0, 0, 1.0).unwrap();
                });
                layer.draw_keyed(NodeId(3), 0, 0, 1.0).unwrap();
            });
            layer.draw_keyed(NodeId(4), 0, 0, 0.0).unwrap();
        });
        assert_eq!(scene.hit_node(5, 5), Ok(Some(NodeId(3))));
        assert_eq!(scene.hit_node(12, 5), Ok(None));
        scene.raise_node(NodeId(1)).unwrap();
        assert_eq!(scene.hit_node(5, 5), Ok(Some(NodeId(1))));
        // Raising a child does not move it outside its immediate scope.
        scene.raise_node(NodeId(2)).unwrap();
        assert_eq!(scene.hit_node(5, 5), Ok(Some(NodeId(1))));
        scene.set_node_opacity(NodeId(1), 0.0).unwrap();
        scene.set_node_opacity(NodeId(3), 0.0).unwrap();
        assert_eq!(scene.hit_node(5, 5), Ok(Some(NodeId(2))));
        assert_eq!(scene.hit_node(6, 5), Ok(None));
        scene.set_node_opacity(NodeId(4), 0.5).unwrap();
        scene.raise_node(NodeId(1)).unwrap();
        assert_eq!(scene.hit_node(5, 5), Ok(Some(NodeId(4))));
        assert_eq!(scene.hit_node(19, 14), Ok(Some(NodeId(4))));
        assert_eq!(scene.hit_node(20, 14), Ok(None));
        assert_eq!(
            scene.raise_node(NodeId(99)),
            Err(NodeError::NotFound(NodeId(99)))
        );
        // Rect containment handles bounds beyond i32::MAX without wrapping.
        assert!(Rect::new(i32::MAX, i32::MAX, 10, 10).contains(i32::MAX, i32::MAX));
        assert!(!Rect::new(0, 0, -1, 2).contains(0, 0));
    }
}
