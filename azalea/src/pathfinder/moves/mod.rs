pub mod basic;
pub mod parkour;

use std::{
    cell::{RefCell, UnsafeCell},
    fmt::Debug,
    sync::Arc,
};

use crate::{JumpEvent, LookAtEvent};

use super::astar;
use azalea_block::BlockState;
use azalea_client::{StartSprintEvent, StartWalkEvent};
use azalea_core::{
    bitset::FixedBitSet,
    position::{BlockPos, ChunkBlockPos, ChunkPos, ChunkSectionBlockPos, ChunkSectionPos, Vec3},
};
use azalea_physics::collision::BlockWithShape;
use azalea_world::Instance;
use bevy_ecs::{entity::Entity, event::EventWriter};
use parking_lot::RwLock;

type Edge = astar::Edge<BlockPos, MoveData>;

pub type SuccessorsFn = fn(&mut Vec<Edge>, &PathfinderCtx, BlockPos);

#[derive(Clone)]
pub struct MoveData {
    /// Use the context to determine what events should be sent to complete this
    /// movement.
    pub execute: &'static (dyn Fn(ExecuteCtx) + Send + Sync),
    /// Whether we've reached the target.
    pub is_reached: &'static (dyn Fn(IsReachedCtx) -> bool + Send + Sync),
}
impl Debug for MoveData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MoveData")
            // .field("move_kind", &self.move_kind)
            .finish()
    }
}

pub struct PathfinderCtx {
    min_y: i32,
    world_lock: Arc<RwLock<Instance>>,
    cached_chunks: RefCell<Vec<(ChunkPos, Vec<azalea_world::Section>)>>,

    cached_block_passable: UnsafeCell<Vec<CachedBlocks>>,
    cached_block_solid: UnsafeCell<Vec<CachedBlocks>>,
}

pub struct CachedBlocks {
    pub pos: ChunkSectionPos,
    pub present: FixedBitSet<4096>,
    pub value: FixedBitSet<4096>,
}

impl PathfinderCtx {
    pub fn new(world_lock: Arc<RwLock<Instance>>) -> Self {
        let min_y = world_lock.read().chunks.min_y;
        Self {
            min_y,
            world_lock,
            cached_chunks: Default::default(),
            cached_block_passable: Default::default(),
            cached_block_solid: Default::default(),
        }
    }

    fn get_block_state(&self, pos: BlockPos) -> Option<BlockState> {
        let chunk_pos = ChunkPos::from(pos);
        let chunk_block_pos = ChunkBlockPos::from(pos);

        let mut cached_chunks = self.cached_chunks.borrow_mut();
        if let Some(sections) = cached_chunks.iter().find_map(|(pos, sections)| {
            if *pos == chunk_pos {
                Some(sections)
            } else {
                None
            }
        }) {
            return azalea_world::chunk_storage::get_block_state_from_sections(
                sections,
                &chunk_block_pos,
                self.min_y,
            );
        }

        let world = self.world_lock.read();
        let chunk = world.chunks.get(&chunk_pos)?;
        let chunk = chunk.read();

        cached_chunks.push((chunk_pos, chunk.sections.clone()));

        azalea_world::chunk_storage::get_block_state_from_sections(
            &chunk.sections,
            &chunk_block_pos,
            self.min_y,
        )
    }

    /// whether this block is passable
    fn uncached_is_block_passable(&self, pos: BlockPos) -> bool {
        let Some(block) = self.get_block_state(pos) else {
            return false;
        };
        if block.is_air() {
            // fast path
            return true;
        }
        if !block.is_shape_empty() {
            return false;
        }
        if block == azalea_registry::Block::Water.into() {
            return false;
        }
        if block.waterlogged() {
            return false;
        }
        // block.waterlogged currently doesn't account for seagrass and some other water
        // blocks
        if block == azalea_registry::Block::Seagrass.into() {
            return false;
        }

        true
    }

    pub fn is_block_passable(&self, pos: BlockPos) -> bool {
        let (section_pos, section_block_pos) =
            (ChunkSectionPos::from(pos), ChunkSectionBlockPos::from(pos));
        let index = u16::from(section_block_pos) as usize;
        // SAFETY: we're only accessing this from one thread
        let cached_block_passable = unsafe { &mut *self.cached_block_passable.get() };
        if let Some(cached) = cached_block_passable.iter_mut().find_map(|cached| {
            if cached.pos == section_pos {
                Some(cached)
            } else {
                None
            }
        }) {
            if cached.present.index(index) {
                return cached.value.index(index);
            } else {
                let passable = self.uncached_is_block_passable(pos);
                cached.present.set(index);
                if passable {
                    cached.value.set(index);
                }
                return passable;
            }
        }

        let passable = self.uncached_is_block_passable(pos);
        let mut present_bitset = FixedBitSet::new();
        let mut value_bitset = FixedBitSet::new();
        present_bitset.set(index);
        if passable {
            value_bitset.set(index);
        }

        cached_block_passable.push(CachedBlocks {
            pos: section_pos,
            present: present_bitset,
            value: value_bitset,
        });
        passable
    }

    /// whether this block has a solid hitbox (i.e. we can stand on it)
    fn uncached_is_block_solid(&self, pos: BlockPos) -> bool {
        let Some(block) = self.get_block_state(pos) else {
            return false;
        };
        if block.is_air() {
            // fast path
            return false;
        }
        block.is_shape_full()
    }

    pub fn is_block_solid(&self, pos: BlockPos) -> bool {
        let (section_pos, section_block_pos) =
            (ChunkSectionPos::from(pos), ChunkSectionBlockPos::from(pos));
        let index = u16::from(section_block_pos) as usize;
        // SAFETY: we're only accessing this from one thread
        let cached_block_solid = unsafe { &mut *self.cached_block_solid.get() };
        if let Some(cached) = cached_block_solid.iter_mut().find_map(|cached| {
            if cached.pos == section_pos {
                Some(cached)
            } else {
                None
            }
        }) {
            if cached.present.index(index) {
                return cached.value.index(index);
            } else {
                let solid = self.uncached_is_block_solid(pos);
                cached.present.set(index);
                if solid {
                    cached.value.set(index);
                }
                return solid;
            }
        }

        let solid = self.uncached_is_block_solid(pos);
        let mut present_bitset = FixedBitSet::new();
        let mut value_bitset = FixedBitSet::new();
        present_bitset.set(index);
        if solid {
            value_bitset.set(index);
        }

        cached_block_solid.push(CachedBlocks {
            pos: section_pos,
            present: present_bitset,
            value: value_bitset,
        });
        solid
    }

    /// Whether this block and the block above are passable
    pub fn is_passable(&self, pos: BlockPos) -> bool {
        self.is_block_passable(pos) && self.is_block_passable(pos.up(1))
    }

    /// Whether we can stand in this position. Checks if the block below is
    /// solid, and that the two blocks above that are passable.

    pub fn is_standable(&self, pos: BlockPos) -> bool {
        self.is_block_solid(pos.down(1)) && self.is_passable(pos)
    }

    /// Get the amount of air blocks until the next solid block below this one.
    pub fn fall_distance(&self, pos: BlockPos) -> u32 {
        let mut distance = 0;
        let mut current_pos = pos.down(1);
        while self.is_block_passable(current_pos) {
            distance += 1;
            current_pos = current_pos.down(1);

            if current_pos.y < self.min_y {
                return u32::MAX;
            }
        }
        distance
    }
}

pub struct ExecuteCtx<'w1, 'w2, 'w3, 'w4, 'a> {
    pub entity: Entity,
    /// The node that we're trying to reach.
    pub target: BlockPos,
    /// The last node that we reached.
    pub start: BlockPos,
    pub position: Vec3,
    pub physics: &'a azalea_entity::Physics,

    pub look_at_events: &'a mut EventWriter<'w1, LookAtEvent>,
    pub sprint_events: &'a mut EventWriter<'w2, StartSprintEvent>,
    pub walk_events: &'a mut EventWriter<'w3, StartWalkEvent>,
    pub jump_events: &'a mut EventWriter<'w4, JumpEvent>,
}
pub struct IsReachedCtx<'a> {
    /// The node that we're trying to reach.
    pub target: BlockPos,
    /// The last node that we reached.
    pub start: BlockPos,
    pub position: Vec3,
    pub physics: &'a azalea_entity::Physics,
}

pub fn default_move(edges: &mut Vec<Edge>, ctx: &PathfinderCtx, node: BlockPos) {
    basic::basic_move(edges, ctx, node);
    parkour::parkour_move(edges, ctx, node);
}

/// Returns whether the entity is at the node and should start going to the
/// next node.
#[must_use]
pub fn default_is_reached(
    IsReachedCtx {
        position, target, ..
    }: IsReachedCtx,
) -> bool {
    BlockPos::from(position) == target
}

#[cfg(test)]
mod tests {
    use super::*;
    use azalea_block::BlockState;
    use azalea_core::position::ChunkPos;
    use azalea_world::{Chunk, ChunkStorage, PartialInstance};

    #[test]
    fn test_is_passable() {
        let mut partial_world = PartialInstance::default();
        let mut world = ChunkStorage::default();

        partial_world
            .chunks
            .set(&ChunkPos { x: 0, z: 0 }, Some(Chunk::default()), &mut world);
        partial_world.chunks.set_block_state(
            &BlockPos::new(0, 0, 0),
            azalea_registry::Block::Stone.into(),
            &world,
        );
        partial_world
            .chunks
            .set_block_state(&BlockPos::new(0, 1, 0), BlockState::AIR, &world);

        let ctx = PathfinderCtx::new(Arc::new(RwLock::new(world.into())));
        assert!(!ctx.is_block_passable(BlockPos::new(0, 0, 0)));
        assert!(ctx.is_block_passable(BlockPos::new(0, 1, 0),));
    }

    #[test]
    fn test_is_solid() {
        let mut partial_world = PartialInstance::default();
        let mut world = ChunkStorage::default();
        partial_world
            .chunks
            .set(&ChunkPos { x: 0, z: 0 }, Some(Chunk::default()), &mut world);
        partial_world.chunks.set_block_state(
            &BlockPos::new(0, 0, 0),
            azalea_registry::Block::Stone.into(),
            &world,
        );
        partial_world
            .chunks
            .set_block_state(&BlockPos::new(0, 1, 0), BlockState::AIR, &world);

        let ctx = PathfinderCtx::new(Arc::new(RwLock::new(world.into())));
        assert!(ctx.is_block_solid(BlockPos::new(0, 0, 0)));
        assert!(!ctx.is_block_solid(BlockPos::new(0, 1, 0)));
    }

    #[test]
    fn test_is_standable() {
        let mut partial_world = PartialInstance::default();
        let mut world = ChunkStorage::default();
        partial_world
            .chunks
            .set(&ChunkPos { x: 0, z: 0 }, Some(Chunk::default()), &mut world);
        partial_world.chunks.set_block_state(
            &BlockPos::new(0, 0, 0),
            azalea_registry::Block::Stone.into(),
            &world,
        );
        partial_world
            .chunks
            .set_block_state(&BlockPos::new(0, 1, 0), BlockState::AIR, &world);
        partial_world
            .chunks
            .set_block_state(&BlockPos::new(0, 2, 0), BlockState::AIR, &world);
        partial_world
            .chunks
            .set_block_state(&BlockPos::new(0, 3, 0), BlockState::AIR, &world);

        let ctx = PathfinderCtx::new(Arc::new(RwLock::new(world.into())));
        assert!(ctx.is_standable(BlockPos::new(0, 1, 0)));
        assert!(!ctx.is_standable(BlockPos::new(0, 0, 0)));
        assert!(!ctx.is_standable(BlockPos::new(0, 2, 0)));
    }
}
