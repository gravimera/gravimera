pub(crate) mod buildings;
pub(crate) mod characters;
pub(crate) mod decorations;
pub(crate) mod effects;
pub(crate) mod projectiles;

use crate::object::registry::ObjectLibrary;

pub(crate) fn register_builtin_objects(library: &mut ObjectLibrary) {
    buildings::register_objects(library);
    characters::register_objects(library);
    projectiles::register_objects(library);
    effects::register_objects(library);
    decorations::register_objects(library);
}
