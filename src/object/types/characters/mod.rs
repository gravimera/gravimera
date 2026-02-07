pub(crate) mod dog;
pub(crate) mod gundam;
pub(crate) mod hero;
pub(crate) mod human;

use crate::object::registry::ObjectLibrary;

pub(crate) fn register_objects(library: &mut ObjectLibrary) {
    library.register(hero::def());
    library.register(dog::def());
    library.register(human::def());
    library.register(gundam::def());
}
