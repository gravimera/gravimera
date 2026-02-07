pub(crate) mod charactor_1;

use crate::object::registry::ObjectLibrary;

pub(crate) fn register_objects(library: &mut ObjectLibrary) {
    library.register(charactor_1::def());
}
