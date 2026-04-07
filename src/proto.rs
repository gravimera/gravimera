#![allow(dead_code)]

pub(crate) mod gravimera {
    pub(crate) mod common {
        pub(crate) mod v1 {
            // Generated protobuf types are checked into the repository so building Gravimera does
            // not require running code generation (or having a protobuf compiler installed).
            include!("proto_gen/gravimera.common.v1.rs");
        }
    }

    pub(crate) mod scene {
        pub(crate) mod v1 {
            include!("proto_gen/gravimera.scene.v1.rs");
        }
    }

    pub(crate) mod terrain {
        pub(crate) mod v1 {
            include!("proto_gen/gravimera.terrain.v1.rs");
        }
    }
}
