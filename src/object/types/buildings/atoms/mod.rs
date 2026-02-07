pub(crate) mod block_slice_0;
pub(crate) mod block_slice_1;
pub(crate) mod block_slice_2;
pub(crate) mod fence_stake;
pub(crate) mod fence_stick;
pub(crate) mod tree_crown_0;
pub(crate) mod tree_crown_1;
pub(crate) mod tree_crown_2;
pub(crate) mod tree_main_0;
pub(crate) mod tree_main_1;
pub(crate) mod tree_main_2;
pub(crate) mod tree_trunk_0;
pub(crate) mod tree_trunk_1;
pub(crate) mod tree_trunk_2;

use crate::object::registry::ObjectLibrary;

pub(crate) fn register_objects(library: &mut ObjectLibrary) {
    library.register(block_slice_0::def());
    library.register(block_slice_1::def());
    library.register(block_slice_2::def());
    library.register(fence_stake::def());
    library.register(fence_stick::def());
    library.register(tree_trunk_0::def());
    library.register(tree_trunk_1::def());
    library.register(tree_trunk_2::def());
    library.register(tree_main_0::def());
    library.register(tree_main_1::def());
    library.register(tree_main_2::def());
    library.register(tree_crown_0::def());
    library.register(tree_crown_1::def());
    library.register(tree_crown_2::def());
}
