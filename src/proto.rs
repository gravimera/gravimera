#![allow(dead_code)]

pub(crate) mod gravimera {
    pub(crate) mod common {
        pub(crate) mod v1 {
            include!(concat!(env!("OUT_DIR"), "/gravimera.common.v1.rs"));
        }
    }

    pub(crate) mod scene {
        pub(crate) mod v1 {
            include!(concat!(env!("OUT_DIR"), "/gravimera.scene.v1.rs"));
        }
    }

    pub(crate) mod terrain {
        pub(crate) mod v1 {
            include!(concat!(env!("OUT_DIR"), "/gravimera.terrain.v1.rs"));
        }
    }
}
