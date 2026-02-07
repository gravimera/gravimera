pub(crate) mod enemy_bullet;
pub(crate) mod gundam_energy_ball;
pub(crate) mod player_bullet;
pub(crate) mod player_shotgun_pellet;

use crate::object::registry::ObjectLibrary;

pub(crate) fn register_objects(library: &mut ObjectLibrary) {
    library.register(player_bullet::def());
    library.register(player_shotgun_pellet::def());
    library.register(enemy_bullet::def());
    library.register(gundam_energy_ball::def());
}
