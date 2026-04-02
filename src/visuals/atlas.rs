// Free-list allocator for physical texture atlas slots.
//
// The atlas is a 3D texture laid out as a 2D grid of tile slots (cols × rows), all sharing
// the same depth dimension (= tile_d). Each slot holds one tile's voxel data.

pub struct AtlasAllocator {
    free_slots: Vec<u32>,
    capacity: usize,
}

impl AtlasAllocator {
    pub fn new(capacity: usize) -> Self {
        // Slots numbered 0..capacity, stored in reverse so pop() gives slot 0 first.
        let free_slots = (0..capacity as u32).rev().collect();
        Self { free_slots, capacity }
    }

    pub fn alloc(&mut self) -> Option<u32> {
        self.free_slots.pop()
    }

    pub fn free(&mut self, slot: u32) {
        debug_assert!((slot as usize) < self.capacity, "atlas slot out of range");
        self.free_slots.push(slot);
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn used(&self) -> usize {
        self.capacity - self.free_slots.len()
    }

    pub fn is_full(&self) -> bool {
        self.free_slots.is_empty()
    }
}
