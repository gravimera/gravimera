import type { GenEnum, GenFile, GenMessage } from "@bufbuild/protobuf/codegenv2";
import type { Uuid128 } from "../../common/v1/uuid_pb.js";
import type { Message } from "@bufbuild/protobuf";
/**
 * Describes the file gravimera/terrain/v1/terrain.proto.
 */
export declare const file_gravimera_terrain_v1_terrain: GenFile;
/**
 * `build/terrain.grav` (per-scene terrain selection + embedded terrain def).
 *
 * @generated from message gravimera.terrain.v1.SceneTerrainDat
 */
export type SceneTerrainDat = Message<"gravimera.terrain.v1.SceneTerrainDat"> & {
    /**
     * v1: terrain_id only
     * v2: terrain_id + embedded terrain_def
     *
     * @generated from field: uint32 format_version = 1;
     */
    formatVersion: number;
    /**
     * Optional: the realm terrain package id. Omitted for Default Terrain.
     *
     * @generated from field: gravimera.common.v1.Uuid128 terrain_id = 2;
     */
    terrainId?: Uuid128;
    /**
     * Optional: embedded terrain definition so the scene can render without the realm package.
     *
     * @generated from field: gravimera.terrain.v1.TerrainDefV1 terrain_def = 3;
     */
    terrainDef?: TerrainDefV1;
};
/**
 * Describes the message gravimera.terrain.v1.SceneTerrainDat.
 * Use `create(SceneTerrainDatSchema)` to create a new message.
 */
export declare const SceneTerrainDatSchema: GenMessage<SceneTerrainDat>;
/**
 * Canonical terrain definition (mirrors `terrain_def_v1.json` / `FloorDefV1`).
 *
 * @generated from message gravimera.terrain.v1.TerrainDefV1
 */
export type TerrainDefV1 = Message<"gravimera.terrain.v1.TerrainDefV1"> & {
    /**
     * @generated from field: uint32 format_version = 1;
     */
    formatVersion: number;
    /**
     * @generated from field: optional string label = 2;
     */
    label?: string;
    /**
     * @generated from field: gravimera.terrain.v1.TerrainMeshV1 mesh = 3;
     */
    mesh?: TerrainMeshV1;
    /**
     * @generated from field: gravimera.terrain.v1.TerrainMaterialV1 material = 4;
     */
    material?: TerrainMaterialV1;
    /**
     * @generated from field: gravimera.terrain.v1.TerrainColoringV1 coloring = 5;
     */
    coloring?: TerrainColoringV1;
    /**
     * @generated from field: gravimera.terrain.v1.TerrainReliefV1 relief = 6;
     */
    relief?: TerrainReliefV1;
    /**
     * @generated from field: gravimera.terrain.v1.TerrainAnimationV1 animation = 7;
     */
    animation?: TerrainAnimationV1;
};
/**
 * Describes the message gravimera.terrain.v1.TerrainDefV1.
 * Use `create(TerrainDefV1Schema)` to create a new message.
 */
export declare const TerrainDefV1Schema: GenMessage<TerrainDefV1>;
/**
 * @generated from message gravimera.terrain.v1.TerrainMeshV1
 */
export type TerrainMeshV1 = Message<"gravimera.terrain.v1.TerrainMeshV1"> & {
    /**
     * @generated from field: gravimera.terrain.v1.TerrainMeshKind kind = 1;
     */
    kind: TerrainMeshKind;
    /**
     * @generated from field: float size_x_m = 2;
     */
    sizeXM: number;
    /**
     * @generated from field: float size_z_m = 3;
     */
    sizeZM: number;
    /**
     * @generated from field: uint32 subdiv_x = 4;
     */
    subdivX: number;
    /**
     * @generated from field: uint32 subdiv_z = 5;
     */
    subdivZ: number;
    /**
     * @generated from field: float thickness_m = 6;
     */
    thicknessM: number;
    /**
     * @generated from field: float uv_tiling_x = 7;
     */
    uvTilingX: number;
    /**
     * @generated from field: float uv_tiling_z = 8;
     */
    uvTilingZ: number;
};
/**
 * Describes the message gravimera.terrain.v1.TerrainMeshV1.
 * Use `create(TerrainMeshV1Schema)` to create a new message.
 */
export declare const TerrainMeshV1Schema: GenMessage<TerrainMeshV1>;
/**
 * @generated from message gravimera.terrain.v1.TerrainMaterialV1
 */
export type TerrainMaterialV1 = Message<"gravimera.terrain.v1.TerrainMaterialV1"> & {
    /**
     * @generated from field: float base_color_r = 1;
     */
    baseColorR: number;
    /**
     * @generated from field: float base_color_g = 2;
     */
    baseColorG: number;
    /**
     * @generated from field: float base_color_b = 3;
     */
    baseColorB: number;
    /**
     * @generated from field: float base_color_a = 4;
     */
    baseColorA: number;
    /**
     * @generated from field: float metallic = 5;
     */
    metallic: number;
    /**
     * @generated from field: float roughness = 6;
     */
    roughness: number;
    /**
     * @generated from field: bool unlit = 7;
     */
    unlit: boolean;
};
/**
 * Describes the message gravimera.terrain.v1.TerrainMaterialV1.
 * Use `create(TerrainMaterialV1Schema)` to create a new message.
 */
export declare const TerrainMaterialV1Schema: GenMessage<TerrainMaterialV1>;
/**
 * @generated from message gravimera.terrain.v1.TerrainNoiseV1
 */
export type TerrainNoiseV1 = Message<"gravimera.terrain.v1.TerrainNoiseV1"> & {
    /**
     * @generated from field: uint32 seed = 1;
     */
    seed: number;
    /**
     * @generated from field: float frequency = 2;
     */
    frequency: number;
    /**
     * @generated from field: uint32 octaves = 3;
     */
    octaves: number;
    /**
     * @generated from field: float lacunarity = 4;
     */
    lacunarity: number;
    /**
     * @generated from field: float gain = 5;
     */
    gain: number;
};
/**
 * Describes the message gravimera.terrain.v1.TerrainNoiseV1.
 * Use `create(TerrainNoiseV1Schema)` to create a new message.
 */
export declare const TerrainNoiseV1Schema: GenMessage<TerrainNoiseV1>;
/**
 * @generated from message gravimera.terrain.v1.TerrainColorRgba
 */
export type TerrainColorRgba = Message<"gravimera.terrain.v1.TerrainColorRgba"> & {
    /**
     * @generated from field: float r = 1;
     */
    r: number;
    /**
     * @generated from field: float g = 2;
     */
    g: number;
    /**
     * @generated from field: float b = 3;
     */
    b: number;
    /**
     * @generated from field: float a = 4;
     */
    a: number;
};
/**
 * Describes the message gravimera.terrain.v1.TerrainColorRgba.
 * Use `create(TerrainColorRgbaSchema)` to create a new message.
 */
export declare const TerrainColorRgbaSchema: GenMessage<TerrainColorRgba>;
/**
 * @generated from message gravimera.terrain.v1.TerrainColoringV1
 */
export type TerrainColoringV1 = Message<"gravimera.terrain.v1.TerrainColoringV1"> & {
    /**
     * @generated from field: gravimera.terrain.v1.TerrainColoringMode mode = 1;
     */
    mode: TerrainColoringMode;
    /**
     * @generated from field: repeated gravimera.terrain.v1.TerrainColorRgba palette = 2;
     */
    palette: TerrainColorRgba[];
    /**
     * @generated from field: float scale_x = 3;
     */
    scaleX: number;
    /**
     * @generated from field: float scale_z = 4;
     */
    scaleZ: number;
    /**
     * @generated from field: float angle_deg = 5;
     */
    angleDeg: number;
    /**
     * @generated from field: gravimera.terrain.v1.TerrainNoiseV1 noise = 6;
     */
    noise?: TerrainNoiseV1;
};
/**
 * Describes the message gravimera.terrain.v1.TerrainColoringV1.
 * Use `create(TerrainColoringV1Schema)` to create a new message.
 */
export declare const TerrainColoringV1Schema: GenMessage<TerrainColoringV1>;
/**
 * @generated from message gravimera.terrain.v1.TerrainReliefV1
 */
export type TerrainReliefV1 = Message<"gravimera.terrain.v1.TerrainReliefV1"> & {
    /**
     * @generated from field: gravimera.terrain.v1.TerrainReliefMode mode = 1;
     */
    mode: TerrainReliefMode;
    /**
     * @generated from field: float amplitude = 2;
     */
    amplitude: number;
    /**
     * @generated from field: gravimera.terrain.v1.TerrainNoiseV1 noise = 3;
     */
    noise?: TerrainNoiseV1;
};
/**
 * Describes the message gravimera.terrain.v1.TerrainReliefV1.
 * Use `create(TerrainReliefV1Schema)` to create a new message.
 */
export declare const TerrainReliefV1Schema: GenMessage<TerrainReliefV1>;
/**
 * @generated from message gravimera.terrain.v1.TerrainWaveV1
 */
export type TerrainWaveV1 = Message<"gravimera.terrain.v1.TerrainWaveV1"> & {
    /**
     * @generated from field: float amplitude = 1;
     */
    amplitude: number;
    /**
     * @generated from field: float wavelength = 2;
     */
    wavelength: number;
    /**
     * @generated from field: float direction_x = 3;
     */
    directionX: number;
    /**
     * @generated from field: float direction_z = 4;
     */
    directionZ: number;
    /**
     * @generated from field: float speed = 5;
     */
    speed: number;
    /**
     * @generated from field: float phase = 6;
     */
    phase: number;
};
/**
 * Describes the message gravimera.terrain.v1.TerrainWaveV1.
 * Use `create(TerrainWaveV1Schema)` to create a new message.
 */
export declare const TerrainWaveV1Schema: GenMessage<TerrainWaveV1>;
/**
 * @generated from message gravimera.terrain.v1.TerrainAnimationV1
 */
export type TerrainAnimationV1 = Message<"gravimera.terrain.v1.TerrainAnimationV1"> & {
    /**
     * @generated from field: gravimera.terrain.v1.TerrainAnimationMode mode = 1;
     */
    mode: TerrainAnimationMode;
    /**
     * @generated from field: repeated gravimera.terrain.v1.TerrainWaveV1 waves = 2;
     */
    waves: TerrainWaveV1[];
    /**
     * @generated from field: float normal_strength = 3;
     */
    normalStrength: number;
};
/**
 * Describes the message gravimera.terrain.v1.TerrainAnimationV1.
 * Use `create(TerrainAnimationV1Schema)` to create a new message.
 */
export declare const TerrainAnimationV1Schema: GenMessage<TerrainAnimationV1>;
/**
 * @generated from enum gravimera.terrain.v1.TerrainMeshKind
 */
export declare enum TerrainMeshKind {
    /**
     * @generated from enum value: TERRAIN_MESH_KIND_GRID = 0;
     */
    GRID = 0
}
/**
 * Describes the enum gravimera.terrain.v1.TerrainMeshKind.
 */
export declare const TerrainMeshKindSchema: GenEnum<TerrainMeshKind>;
/**
 * @generated from enum gravimera.terrain.v1.TerrainColoringMode
 */
export declare enum TerrainColoringMode {
    /**
     * @generated from enum value: TERRAIN_COLORING_MODE_SOLID = 0;
     */
    SOLID = 0,
    /**
     * @generated from enum value: TERRAIN_COLORING_MODE_CHECKER = 1;
     */
    CHECKER = 1,
    /**
     * @generated from enum value: TERRAIN_COLORING_MODE_STRIPES = 2;
     */
    STRIPES = 2,
    /**
     * @generated from enum value: TERRAIN_COLORING_MODE_GRADIENT = 3;
     */
    GRADIENT = 3,
    /**
     * @generated from enum value: TERRAIN_COLORING_MODE_NOISE = 4;
     */
    NOISE = 4
}
/**
 * Describes the enum gravimera.terrain.v1.TerrainColoringMode.
 */
export declare const TerrainColoringModeSchema: GenEnum<TerrainColoringMode>;
/**
 * @generated from enum gravimera.terrain.v1.TerrainReliefMode
 */
export declare enum TerrainReliefMode {
    /**
     * @generated from enum value: TERRAIN_RELIEF_MODE_NONE = 0;
     */
    NONE = 0,
    /**
     * @generated from enum value: TERRAIN_RELIEF_MODE_NOISE = 1;
     */
    NOISE = 1
}
/**
 * Describes the enum gravimera.terrain.v1.TerrainReliefMode.
 */
export declare const TerrainReliefModeSchema: GenEnum<TerrainReliefMode>;
/**
 * @generated from enum gravimera.terrain.v1.TerrainAnimationMode
 */
export declare enum TerrainAnimationMode {
    /**
     * @generated from enum value: TERRAIN_ANIMATION_MODE_NONE = 0;
     */
    NONE = 0,
    /**
     * @generated from enum value: TERRAIN_ANIMATION_MODE_CPU = 1;
     */
    CPU = 1,
    /**
     * @generated from enum value: TERRAIN_ANIMATION_MODE_GPU = 2;
     */
    GPU = 2
}
/**
 * Describes the enum gravimera.terrain.v1.TerrainAnimationMode.
 */
export declare const TerrainAnimationModeSchema: GenEnum<TerrainAnimationMode>;
