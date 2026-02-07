pub(crate) mod blood_particle;
pub(crate) mod energy_impact_particle;
pub(crate) mod explosion_particle;
pub(crate) mod laser;
pub(crate) mod move_target_marker;

use crate::object::registry::ObjectLibrary;

pub(crate) fn register_objects(library: &mut ObjectLibrary) {
    library.register(laser::def());
    library.register(move_target_marker::def());
    library.register(explosion_particle::def());
    library.register(blood_particle::def());
    library.register(energy_impact_particle::def());
}
