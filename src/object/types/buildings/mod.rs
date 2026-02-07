pub(crate) mod atoms;
pub(crate) mod block;
pub(crate) mod fence_x;
pub(crate) mod fence_z;
pub(crate) mod ground;
pub(crate) mod tree_huge;
pub(crate) mod tree_medium;
pub(crate) mod tree_small;

use crate::object::registry::ObjectLibrary;
use crate::types::{BuildObjectKind, BuildPreviewSpec, FenceAxis};

pub(crate) fn register_objects(library: &mut ObjectLibrary) {
    atoms::register_objects(library);
    library.register(block::def());
    library.register(fence_x::def());
    library.register(fence_z::def());
    library.register(ground::def());
    library.register(tree_small::def());
    library.register(tree_medium::def());
    library.register(tree_huge::def());
}

pub(crate) fn prefab_id_from_build_spec(spec: BuildPreviewSpec) -> u128 {
    match spec.kind {
        BuildObjectKind::Block => block::object_id(),
        BuildObjectKind::Fence => match spec.fence_axis {
            FenceAxis::X => fence_x::object_id(),
            FenceAxis::Z => fence_z::object_id(),
        },
        BuildObjectKind::Tree => match spec.tree_variant % 3 {
            0 => tree_small::object_id(),
            1 => tree_medium::object_id(),
            _ => tree_huge::object_id(),
        },
    }
}

pub(crate) fn build_spec_from_prefab_id(object_id: u128) -> Option<BuildPreviewSpec> {
    if object_id == block::object_id() {
        return Some(BuildPreviewSpec {
            kind: BuildObjectKind::Block,
            fence_axis: FenceAxis::X,
            tree_variant: 0,
        });
    }
    if object_id == fence_x::object_id() {
        return Some(BuildPreviewSpec {
            kind: BuildObjectKind::Fence,
            fence_axis: FenceAxis::X,
            tree_variant: 0,
        });
    }
    if object_id == fence_z::object_id() {
        return Some(BuildPreviewSpec {
            kind: BuildObjectKind::Fence,
            fence_axis: FenceAxis::Z,
            tree_variant: 0,
        });
    }
    if object_id == tree_small::object_id() {
        return Some(BuildPreviewSpec {
            kind: BuildObjectKind::Tree,
            fence_axis: FenceAxis::X,
            tree_variant: 0,
        });
    }
    if object_id == tree_medium::object_id() {
        return Some(BuildPreviewSpec {
            kind: BuildObjectKind::Tree,
            fence_axis: FenceAxis::X,
            tree_variant: 1,
        });
    }
    if object_id == tree_huge::object_id() {
        return Some(BuildPreviewSpec {
            kind: BuildObjectKind::Tree,
            fence_axis: FenceAxis::X,
            tree_variant: 2,
        });
    }

    None
}

pub(crate) fn fence_axis_from_prefab_id(object_id: u128) -> Option<FenceAxis> {
    let spec = build_spec_from_prefab_id(object_id)?;
    if spec.kind == BuildObjectKind::Fence {
        Some(spec.fence_axis)
    } else {
        None
    }
}
