import type { GenEnum, GenFile, GenMessage } from "@bufbuild/protobuf/codegenv2";
import type { Uuid128 } from "../../common/v1/uuid_pb.js";
import type { Message } from "@bufbuild/protobuf";
/**
 * Describes the file gravimera/scene/v1/scene.proto.
 */
export declare const file_gravimera_scene_v1_scene: GenFile;
/**
 * `build/scene.grav` (per-scene embedded prefab defs + build instances).
 *
 * @generated from message gravimera.scene.v1.SceneDat
 */
export type SceneDat = Message<"gravimera.scene.v1.SceneDat"> & {
    /**
     * @generated from field: uint32 version = 1;
     */
    version: number;
    /**
     * @generated from field: uint32 units_per_meter = 2;
     */
    unitsPerMeter: number;
    /**
     * @generated from field: repeated gravimera.scene.v1.SceneDatObjectDef defs = 3;
     */
    defs: SceneDatObjectDef[];
    /**
     * @generated from field: repeated gravimera.scene.v1.SceneDatObjectInstance instances = 4;
     */
    instances: SceneDatObjectInstance[];
};
/**
 * Describes the message gravimera.scene.v1.SceneDat.
 * Use `create(SceneDatSchema)` to create a new message.
 */
export declare const SceneDatSchema: GenMessage<SceneDat>;
/**
 * @generated from message gravimera.scene.v1.Float32Dat
 */
export type Float32Dat = Message<"gravimera.scene.v1.Float32Dat"> & {
    /**
     * @generated from field: float value = 1;
     */
    value: number;
};
/**
 * Describes the message gravimera.scene.v1.Float32Dat.
 * Use `create(Float32DatSchema)` to create a new message.
 */
export declare const Float32DatSchema: GenMessage<Float32Dat>;
/**
 * @generated from message gravimera.scene.v1.ColorDat
 */
export type ColorDat = Message<"gravimera.scene.v1.ColorDat"> & {
    /**
     * @generated from field: fixed32 rgba = 1;
     */
    rgba: number;
};
/**
 * Describes the message gravimera.scene.v1.ColorDat.
 * Use `create(ColorDatSchema)` to create a new message.
 */
export declare const ColorDatSchema: GenMessage<ColorDat>;
/**
 * @generated from message gravimera.scene.v1.EmptyDat
 */
export type EmptyDat = Message<"gravimera.scene.v1.EmptyDat"> & {};
/**
 * Describes the message gravimera.scene.v1.EmptyDat.
 * Use `create(EmptyDatSchema)` to create a new message.
 */
export declare const EmptyDatSchema: GenMessage<EmptyDat>;
/**
 * @generated from message gravimera.scene.v1.SceneDatObjectInstance
 */
export type SceneDatObjectInstance = Message<"gravimera.scene.v1.SceneDatObjectInstance"> & {
    /**
     * @generated from field: gravimera.common.v1.Uuid128 instance_id = 1;
     */
    instanceId?: Uuid128;
    /**
     * @generated from field: gravimera.common.v1.Uuid128 base_object_id = 2;
     */
    baseObjectId?: Uuid128;
    /**
     * @generated from field: int32 x_units = 3;
     */
    xUnits: number;
    /**
     * @generated from field: int32 y_units = 4;
     */
    yUnits: number;
    /**
     * @generated from field: int32 z_units = 5;
     */
    zUnits: number;
    /**
     * @generated from field: float rot_x = 6;
     */
    rotX: number;
    /**
     * @generated from field: float rot_y = 7;
     */
    rotY: number;
    /**
     * @generated from field: float rot_z = 8;
     */
    rotZ: number;
    /**
     * @generated from field: float rot_w = 9;
     */
    rotW: number;
    /**
     * @generated from field: gravimera.scene.v1.ColorDat tint = 10;
     */
    tint?: ColorDat;
    /**
     * @generated from field: gravimera.scene.v1.Float32Dat scale_x = 11;
     */
    scaleX?: Float32Dat;
    /**
     * @generated from field: gravimera.scene.v1.Float32Dat scale_y = 12;
     */
    scaleY?: Float32Dat;
    /**
     * @generated from field: gravimera.scene.v1.Float32Dat scale_z = 13;
     */
    scaleZ?: Float32Dat;
    /**
     * @generated from field: repeated gravimera.common.v1.Uuid128 forms = 14;
     */
    forms: Uuid128[];
    /**
     * @generated from field: uint32 active_form = 15;
     */
    activeForm: number;
    /**
     * Note: tag gap 16..18 reserved for removed fields.
     *
     * @generated from field: bool is_protagonist = 19;
     */
    isProtagonist: boolean;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatObjectInstance.
 * Use `create(SceneDatObjectInstanceSchema)` to create a new message.
 */
export declare const SceneDatObjectInstanceSchema: GenMessage<SceneDatObjectInstance>;
/**
 * @generated from message gravimera.scene.v1.SceneDatObjectDef
 */
export type SceneDatObjectDef = Message<"gravimera.scene.v1.SceneDatObjectDef"> & {
    /**
     * @generated from field: gravimera.common.v1.Uuid128 object_id = 1;
     */
    objectId?: Uuid128;
    /**
     * @generated from field: string label = 2;
     */
    label: string;
    /**
     * @generated from field: float size_x = 3;
     */
    sizeX: number;
    /**
     * @generated from field: float size_y = 4;
     */
    sizeY: number;
    /**
     * @generated from field: float size_z = 5;
     */
    sizeZ: number;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatCollider collider = 6;
     */
    collider?: SceneDatCollider;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatInteraction interaction = 7;
     */
    interaction?: SceneDatInteraction;
    /**
     * @generated from field: repeated gravimera.scene.v1.SceneDatPartDef parts = 8;
     */
    parts: SceneDatPartDef[];
    /**
     * @generated from field: gravimera.scene.v1.ColorDat minimap_color = 9;
     */
    minimapColor?: ColorDat;
    /**
     * @generated from field: gravimera.scene.v1.Float32Dat health_bar_offset_y = 10;
     */
    healthBarOffsetY?: Float32Dat;
    /**
     * @generated from field: repeated gravimera.scene.v1.SceneDatAnchorDef anchors = 11;
     */
    anchors: SceneDatAnchorDef[];
    /**
     * @generated from field: gravimera.scene.v1.SceneDatMobility mobility = 12;
     */
    mobility?: SceneDatMobility;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatProjectile projectile = 13;
     */
    projectile?: SceneDatProjectile;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatUnitAttack attack = 14;
     */
    attack?: SceneDatUnitAttack;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatAimProfile aim = 15;
     */
    aim?: SceneDatAimProfile;
    /**
     * @generated from field: gravimera.scene.v1.Float32Dat ground_origin_y = 16;
     */
    groundOriginY?: Float32Dat;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatObjectDef.
 * Use `create(SceneDatObjectDefSchema)` to create a new message.
 */
export declare const SceneDatObjectDefSchema: GenMessage<SceneDatObjectDef>;
/**
 * @generated from message gravimera.scene.v1.SceneDatAimProfile
 */
export type SceneDatAimProfile = Message<"gravimera.scene.v1.SceneDatAimProfile"> & {
    /**
     * @generated from field: gravimera.scene.v1.Float32Dat max_yaw_delta_degrees = 1;
     */
    maxYawDeltaDegrees?: Float32Dat;
    /**
     * @generated from field: repeated gravimera.common.v1.Uuid128 components = 2;
     */
    components: Uuid128[];
};
/**
 * Describes the message gravimera.scene.v1.SceneDatAimProfile.
 * Use `create(SceneDatAimProfileSchema)` to create a new message.
 */
export declare const SceneDatAimProfileSchema: GenMessage<SceneDatAimProfile>;
/**
 * @generated from message gravimera.scene.v1.SceneDatMobility
 */
export type SceneDatMobility = Message<"gravimera.scene.v1.SceneDatMobility"> & {
    /**
     * @generated from field: gravimera.scene.v1.SceneDatMobilityMode mode = 1;
     */
    mode: SceneDatMobilityMode;
    /**
     * @generated from field: float max_speed = 2;
     */
    maxSpeed: number;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatMobility.
 * Use `create(SceneDatMobilitySchema)` to create a new message.
 */
export declare const SceneDatMobilitySchema: GenMessage<SceneDatMobility>;
/**
 * @generated from message gravimera.scene.v1.SceneDatProjectile
 */
export type SceneDatProjectile = Message<"gravimera.scene.v1.SceneDatProjectile"> & {
    /**
     * @generated from field: gravimera.scene.v1.SceneDatProjectileObstacleRule obstacle_rule = 1;
     */
    obstacleRule: SceneDatProjectileObstacleRule;
    /**
     * @generated from field: float speed = 2;
     */
    speed: number;
    /**
     * @generated from field: float ttl_secs = 3;
     */
    ttlSecs: number;
    /**
     * @generated from field: int32 damage = 4;
     */
    damage: number;
    /**
     * @generated from field: bool spawn_energy_impact = 5;
     */
    spawnEnergyImpact: boolean;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatProjectile.
 * Use `create(SceneDatProjectileSchema)` to create a new message.
 */
export declare const SceneDatProjectileSchema: GenMessage<SceneDatProjectile>;
/**
 * @generated from message gravimera.scene.v1.SceneDatUnitAttack
 */
export type SceneDatUnitAttack = Message<"gravimera.scene.v1.SceneDatUnitAttack"> & {
    /**
     * @generated from field: gravimera.scene.v1.SceneDatUnitAttackKind kind = 1;
     */
    kind: SceneDatUnitAttackKind;
    /**
     * @generated from field: float cooldown_secs = 2;
     */
    cooldownSecs: number;
    /**
     * @generated from field: int32 damage = 3;
     */
    damage: number;
    /**
     * @generated from field: float anim_window_secs = 4;
     */
    animWindowSecs: number;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatMeleeAttack melee = 5;
     */
    melee?: SceneDatMeleeAttack;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatRangedAttack ranged = 6;
     */
    ranged?: SceneDatRangedAttack;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatUnitAttack.
 * Use `create(SceneDatUnitAttackSchema)` to create a new message.
 */
export declare const SceneDatUnitAttackSchema: GenMessage<SceneDatUnitAttack>;
/**
 * @generated from message gravimera.scene.v1.SceneDatMeleeAttack
 */
export type SceneDatMeleeAttack = Message<"gravimera.scene.v1.SceneDatMeleeAttack"> & {
    /**
     * @generated from field: float range = 1;
     */
    range: number;
    /**
     * @generated from field: float radius = 2;
     */
    radius: number;
    /**
     * @generated from field: float arc_degrees = 3;
     */
    arcDegrees: number;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatMeleeAttack.
 * Use `create(SceneDatMeleeAttackSchema)` to create a new message.
 */
export declare const SceneDatMeleeAttackSchema: GenMessage<SceneDatMeleeAttack>;
/**
 * @generated from message gravimera.scene.v1.SceneDatRangedAttack
 */
export type SceneDatRangedAttack = Message<"gravimera.scene.v1.SceneDatRangedAttack"> & {
    /**
     * @generated from field: gravimera.common.v1.Uuid128 projectile_prefab = 1;
     */
    projectilePrefab?: Uuid128;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatAnchorRef muzzle = 2;
     */
    muzzle?: SceneDatAnchorRef;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatRangedAttack.
 * Use `create(SceneDatRangedAttackSchema)` to create a new message.
 */
export declare const SceneDatRangedAttackSchema: GenMessage<SceneDatRangedAttack>;
/**
 * @generated from message gravimera.scene.v1.SceneDatAnchorRef
 */
export type SceneDatAnchorRef = Message<"gravimera.scene.v1.SceneDatAnchorRef"> & {
    /**
     * @generated from field: gravimera.common.v1.Uuid128 object_id = 1;
     */
    objectId?: Uuid128;
    /**
     * @generated from field: string anchor = 2;
     */
    anchor: string;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatAnchorRef.
 * Use `create(SceneDatAnchorRefSchema)` to create a new message.
 */
export declare const SceneDatAnchorRefSchema: GenMessage<SceneDatAnchorRef>;
/**
 * @generated from message gravimera.scene.v1.SceneDatAnchorDef
 */
export type SceneDatAnchorDef = Message<"gravimera.scene.v1.SceneDatAnchorDef"> & {
    /**
     * @generated from field: string name = 1;
     */
    name: string;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatTransform transform = 2;
     */
    transform?: SceneDatTransform;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatAnchorDef.
 * Use `create(SceneDatAnchorDefSchema)` to create a new message.
 */
export declare const SceneDatAnchorDefSchema: GenMessage<SceneDatAnchorDef>;
/**
 * @generated from message gravimera.scene.v1.SceneDatCollider
 */
export type SceneDatCollider = Message<"gravimera.scene.v1.SceneDatCollider"> & {
    /**
     * @generated from oneof gravimera.scene.v1.SceneDatCollider.kind
     */
    kind: {
        /**
         * @generated from field: gravimera.scene.v1.EmptyDat none = 1;
         */
        value: EmptyDat;
        case: "none";
    } | {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatCircleXz circle_xz = 2;
         */
        value: SceneDatCircleXz;
        case: "circleXz";
    } | {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatAabbXz aabb_xz = 3;
         */
        value: SceneDatAabbXz;
        case: "aabbXz";
    } | {
        case: undefined;
        value?: undefined;
    };
};
/**
 * Describes the message gravimera.scene.v1.SceneDatCollider.
 * Use `create(SceneDatColliderSchema)` to create a new message.
 */
export declare const SceneDatColliderSchema: GenMessage<SceneDatCollider>;
/**
 * @generated from message gravimera.scene.v1.SceneDatCircleXz
 */
export type SceneDatCircleXz = Message<"gravimera.scene.v1.SceneDatCircleXz"> & {
    /**
     * @generated from field: float radius = 1;
     */
    radius: number;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatCircleXz.
 * Use `create(SceneDatCircleXzSchema)` to create a new message.
 */
export declare const SceneDatCircleXzSchema: GenMessage<SceneDatCircleXz>;
/**
 * @generated from message gravimera.scene.v1.SceneDatAabbXz
 */
export type SceneDatAabbXz = Message<"gravimera.scene.v1.SceneDatAabbXz"> & {
    /**
     * @generated from field: float half_x = 1;
     */
    halfX: number;
    /**
     * @generated from field: float half_z = 2;
     */
    halfZ: number;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatAabbXz.
 * Use `create(SceneDatAabbXzSchema)` to create a new message.
 */
export declare const SceneDatAabbXzSchema: GenMessage<SceneDatAabbXz>;
/**
 * @generated from message gravimera.scene.v1.SceneDatInteraction
 */
export type SceneDatInteraction = Message<"gravimera.scene.v1.SceneDatInteraction"> & {
    /**
     * @generated from field: bool blocks_bullets = 1;
     */
    blocksBullets: boolean;
    /**
     * @generated from field: bool blocks_laser = 2;
     */
    blocksLaser: boolean;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatMovementBlock movement_block = 3;
     */
    movementBlock?: SceneDatMovementBlock;
    /**
     * @generated from field: bool supports_standing = 4;
     */
    supportsStanding: boolean;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatInteraction.
 * Use `create(SceneDatInteractionSchema)` to create a new message.
 */
export declare const SceneDatInteractionSchema: GenMessage<SceneDatInteraction>;
/**
 * @generated from message gravimera.scene.v1.SceneDatMovementBlock
 */
export type SceneDatMovementBlock = Message<"gravimera.scene.v1.SceneDatMovementBlock"> & {
    /**
     * @generated from oneof gravimera.scene.v1.SceneDatMovementBlock.kind
     */
    kind: {
        /**
         * @generated from field: gravimera.scene.v1.EmptyDat always = 1;
         */
        value: EmptyDat;
        case: "always";
    } | {
        /**
         * @generated from field: gravimera.scene.v1.Float32Dat upper_body_fraction = 2;
         */
        value: Float32Dat;
        case: "upperBodyFraction";
    } | {
        case: undefined;
        value?: undefined;
    };
};
/**
 * Describes the message gravimera.scene.v1.SceneDatMovementBlock.
 * Use `create(SceneDatMovementBlockSchema)` to create a new message.
 */
export declare const SceneDatMovementBlockSchema: GenMessage<SceneDatMovementBlock>;
/**
 * @generated from message gravimera.scene.v1.SceneDatPartDef
 */
export type SceneDatPartDef = Message<"gravimera.scene.v1.SceneDatPartDef"> & {
    /**
     * @generated from field: gravimera.common.v1.Uuid128 part_id = 1;
     */
    partId?: Uuid128;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatTransform transform = 2;
     */
    transform?: SceneDatTransform;
    /**
     * @generated from oneof gravimera.scene.v1.SceneDatPartDef.kind
     */
    kind: {
        /**
         * @generated from field: gravimera.common.v1.Uuid128 object_ref = 3;
         */
        value: Uuid128;
        case: "objectRef";
    } | {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatPrimitive primitive = 4;
         */
        value: SceneDatPrimitive;
        case: "primitive";
    } | {
        /**
         * @generated from field: string model = 5;
         */
        value: string;
        case: "model";
    } | {
        case: undefined;
        value?: undefined;
    };
    /**
     * @generated from field: gravimera.scene.v1.SceneDatAttachment attachment = 6;
     */
    attachment?: SceneDatAttachment;
    /**
     * @generated from field: repeated gravimera.scene.v1.SceneDatPartAnimationSlot animations = 7;
     */
    animations: SceneDatPartAnimationSlot[];
    /**
     * @generated from field: gravimera.scene.v1.SceneDatTransform fallback_basis = 8;
     */
    fallbackBasis?: SceneDatTransform;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatPartDef.
 * Use `create(SceneDatPartDefSchema)` to create a new message.
 */
export declare const SceneDatPartDefSchema: GenMessage<SceneDatPartDef>;
/**
 * @generated from message gravimera.scene.v1.SceneDatAttachment
 */
export type SceneDatAttachment = Message<"gravimera.scene.v1.SceneDatAttachment"> & {
    /**
     * @generated from field: string parent_anchor = 1;
     */
    parentAnchor: string;
    /**
     * @generated from field: string child_anchor = 2;
     */
    childAnchor: string;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatAttachment.
 * Use `create(SceneDatAttachmentSchema)` to create a new message.
 */
export declare const SceneDatAttachmentSchema: GenMessage<SceneDatAttachment>;
/**
 * @generated from message gravimera.scene.v1.SceneDatPartAnimationSlot
 */
export type SceneDatPartAnimationSlot = Message<"gravimera.scene.v1.SceneDatPartAnimationSlot"> & {
    /**
     * @generated from field: string channel = 1;
     */
    channel: string;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatPartAnimation animation = 2;
     */
    animation?: SceneDatPartAnimation;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatPartAnimationFamily family = 3;
     */
    family: SceneDatPartAnimationFamily;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatPartAnimationSlot.
 * Use `create(SceneDatPartAnimationSlotSchema)` to create a new message.
 */
export declare const SceneDatPartAnimationSlotSchema: GenMessage<SceneDatPartAnimationSlot>;
/**
 * @generated from message gravimera.scene.v1.SceneDatPartAnimation
 */
export type SceneDatPartAnimation = Message<"gravimera.scene.v1.SceneDatPartAnimation"> & {
    /**
     * NOTE: keep tags disjoint from the non-oneof fields below (driver/speed/time_offset/basis).
     *
     * @generated from oneof gravimera.scene.v1.SceneDatPartAnimation.kind
     */
    kind: {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatPartAnimationLoop loop = 1;
         */
        value: SceneDatPartAnimationLoop;
        case: "loop";
    } | {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatPartAnimationSpin spin = 2;
         */
        value: SceneDatPartAnimationSpin;
        case: "spin";
    } | {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatPartAnimationLoop once = 6;
         */
        value: SceneDatPartAnimationLoop;
        case: "once";
    } | {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatPartAnimationLoop ping_pong = 7;
         */
        value: SceneDatPartAnimationLoop;
        case: "pingPong";
    } | {
        case: undefined;
        value?: undefined;
    };
    /**
     * @generated from field: gravimera.scene.v1.SceneDatPartAnimationDriver driver = 3;
     */
    driver: SceneDatPartAnimationDriver;
    /**
     * @generated from field: float speed_scale = 4;
     */
    speedScale: number;
    /**
     * @generated from field: float time_offset_units = 5;
     */
    timeOffsetUnits: number;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatTransform basis = 8;
     */
    basis?: SceneDatTransform;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatPartAnimation.
 * Use `create(SceneDatPartAnimationSchema)` to create a new message.
 */
export declare const SceneDatPartAnimationSchema: GenMessage<SceneDatPartAnimation>;
/**
 * @generated from message gravimera.scene.v1.SceneDatPartAnimationLoop
 */
export type SceneDatPartAnimationLoop = Message<"gravimera.scene.v1.SceneDatPartAnimationLoop"> & {
    /**
     * @generated from field: float duration_secs = 1;
     */
    durationSecs: number;
    /**
     * @generated from field: repeated gravimera.scene.v1.SceneDatPartAnimationKeyframe keyframes = 2;
     */
    keyframes: SceneDatPartAnimationKeyframe[];
};
/**
 * Describes the message gravimera.scene.v1.SceneDatPartAnimationLoop.
 * Use `create(SceneDatPartAnimationLoopSchema)` to create a new message.
 */
export declare const SceneDatPartAnimationLoopSchema: GenMessage<SceneDatPartAnimationLoop>;
/**
 * @generated from message gravimera.scene.v1.SceneDatPartAnimationSpin
 */
export type SceneDatPartAnimationSpin = Message<"gravimera.scene.v1.SceneDatPartAnimationSpin"> & {
    /**
     * @generated from field: float axis_x = 1;
     */
    axisX: number;
    /**
     * @generated from field: float axis_y = 2;
     */
    axisY: number;
    /**
     * @generated from field: float axis_z = 3;
     */
    axisZ: number;
    /**
     * @generated from field: float radians_per_unit = 4;
     */
    radiansPerUnit: number;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatPartAnimationSpinAxisSpace axis_space = 5;
     */
    axisSpace: SceneDatPartAnimationSpinAxisSpace;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatPartAnimationSpin.
 * Use `create(SceneDatPartAnimationSpinSchema)` to create a new message.
 */
export declare const SceneDatPartAnimationSpinSchema: GenMessage<SceneDatPartAnimationSpin>;
/**
 * @generated from message gravimera.scene.v1.SceneDatPartAnimationKeyframe
 */
export type SceneDatPartAnimationKeyframe = Message<"gravimera.scene.v1.SceneDatPartAnimationKeyframe"> & {
    /**
     * @generated from field: float time_secs = 1;
     */
    timeSecs: number;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatTransform delta = 2;
     */
    delta?: SceneDatTransform;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatPartAnimationKeyframe.
 * Use `create(SceneDatPartAnimationKeyframeSchema)` to create a new message.
 */
export declare const SceneDatPartAnimationKeyframeSchema: GenMessage<SceneDatPartAnimationKeyframe>;
/**
 * @generated from message gravimera.scene.v1.SceneDatTransform
 */
export type SceneDatTransform = Message<"gravimera.scene.v1.SceneDatTransform"> & {
    /**
     * @generated from field: float tx = 1;
     */
    tx: number;
    /**
     * @generated from field: float ty = 2;
     */
    ty: number;
    /**
     * @generated from field: float tz = 3;
     */
    tz: number;
    /**
     * @generated from field: float rx = 4;
     */
    rx: number;
    /**
     * @generated from field: float ry = 5;
     */
    ry: number;
    /**
     * @generated from field: float rz = 6;
     */
    rz: number;
    /**
     * @generated from field: float rw = 7;
     */
    rw: number;
    /**
     * @generated from field: float sx = 8;
     */
    sx: number;
    /**
     * @generated from field: float sy = 9;
     */
    sy: number;
    /**
     * @generated from field: float sz = 10;
     */
    sz: number;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatTransform.
 * Use `create(SceneDatTransformSchema)` to create a new message.
 */
export declare const SceneDatTransformSchema: GenMessage<SceneDatTransform>;
/**
 * @generated from message gravimera.scene.v1.SceneDatPrimitive
 */
export type SceneDatPrimitive = Message<"gravimera.scene.v1.SceneDatPrimitive"> & {
    /**
     * @generated from oneof gravimera.scene.v1.SceneDatPrimitive.kind
     */
    kind: {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatPrimitiveMeshRef mesh_ref = 1;
         */
        value: SceneDatPrimitiveMeshRef;
        case: "meshRef";
    } | {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatPrimitiveSolid solid = 2;
         */
        value: SceneDatPrimitiveSolid;
        case: "solid";
    } | {
        case: undefined;
        value?: undefined;
    };
};
/**
 * Describes the message gravimera.scene.v1.SceneDatPrimitive.
 * Use `create(SceneDatPrimitiveSchema)` to create a new message.
 */
export declare const SceneDatPrimitiveSchema: GenMessage<SceneDatPrimitive>;
/**
 * @generated from message gravimera.scene.v1.SceneDatPrimitiveMeshRef
 */
export type SceneDatPrimitiveMeshRef = Message<"gravimera.scene.v1.SceneDatPrimitiveMeshRef"> & {
    /**
     * @generated from field: gravimera.scene.v1.SceneDatMeshKey mesh = 1;
     */
    mesh: SceneDatMeshKey;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatMaterialKey material = 2;
     */
    material?: SceneDatMaterialKey;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatPrimitiveMeshRef.
 * Use `create(SceneDatPrimitiveMeshRefSchema)` to create a new message.
 */
export declare const SceneDatPrimitiveMeshRefSchema: GenMessage<SceneDatPrimitiveMeshRef>;
/**
 * @generated from message gravimera.scene.v1.SceneDatPrimitiveSolid
 */
export type SceneDatPrimitiveSolid = Message<"gravimera.scene.v1.SceneDatPrimitiveSolid"> & {
    /**
     * @generated from field: gravimera.scene.v1.SceneDatMeshKey mesh = 1;
     */
    mesh: SceneDatMeshKey;
    /**
     * @generated from field: gravimera.scene.v1.SceneDatPrimitiveParams params = 2;
     */
    params?: SceneDatPrimitiveParams;
    /**
     * @generated from field: gravimera.scene.v1.ColorDat color = 3;
     */
    color?: ColorDat;
    /**
     * @generated from field: bool unlit = 4;
     */
    unlit: boolean;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatPrimitiveSolid.
 * Use `create(SceneDatPrimitiveSolidSchema)` to create a new message.
 */
export declare const SceneDatPrimitiveSolidSchema: GenMessage<SceneDatPrimitiveSolid>;
/**
 * @generated from message gravimera.scene.v1.SceneDatMaterialKey
 */
export type SceneDatMaterialKey = Message<"gravimera.scene.v1.SceneDatMaterialKey"> & {
    /**
     * @generated from oneof gravimera.scene.v1.SceneDatMaterialKey.kind
     */
    kind: {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatBuildBlock build_block = 1;
         */
        value: SceneDatBuildBlock;
        case: "buildBlock";
    } | {
        /**
         * @generated from field: gravimera.scene.v1.EmptyDat fence_stake = 2;
         */
        value: EmptyDat;
        case: "fenceStake";
    } | {
        /**
         * @generated from field: gravimera.scene.v1.EmptyDat fence_stick = 3;
         */
        value: EmptyDat;
        case: "fenceStick";
    } | {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatTreeVariant tree_trunk = 4;
         */
        value: SceneDatTreeVariant;
        case: "treeTrunk";
    } | {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatTreeVariant tree_main = 5;
         */
        value: SceneDatTreeVariant;
        case: "treeMain";
    } | {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatTreeVariant tree_crown = 6;
         */
        value: SceneDatTreeVariant;
        case: "treeCrown";
    } | {
        case: undefined;
        value?: undefined;
    };
};
/**
 * Describes the message gravimera.scene.v1.SceneDatMaterialKey.
 * Use `create(SceneDatMaterialKeySchema)` to create a new message.
 */
export declare const SceneDatMaterialKeySchema: GenMessage<SceneDatMaterialKey>;
/**
 * @generated from message gravimera.scene.v1.SceneDatBuildBlock
 */
export type SceneDatBuildBlock = Message<"gravimera.scene.v1.SceneDatBuildBlock"> & {
    /**
     * @generated from field: uint32 index = 1;
     */
    index: number;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatBuildBlock.
 * Use `create(SceneDatBuildBlockSchema)` to create a new message.
 */
export declare const SceneDatBuildBlockSchema: GenMessage<SceneDatBuildBlock>;
/**
 * @generated from message gravimera.scene.v1.SceneDatTreeVariant
 */
export type SceneDatTreeVariant = Message<"gravimera.scene.v1.SceneDatTreeVariant"> & {
    /**
     * @generated from field: uint32 variant = 1;
     */
    variant: number;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatTreeVariant.
 * Use `create(SceneDatTreeVariantSchema)` to create a new message.
 */
export declare const SceneDatTreeVariantSchema: GenMessage<SceneDatTreeVariant>;
/**
 * @generated from message gravimera.scene.v1.SceneDatPrimitiveParams
 */
export type SceneDatPrimitiveParams = Message<"gravimera.scene.v1.SceneDatPrimitiveParams"> & {
    /**
     * @generated from oneof gravimera.scene.v1.SceneDatPrimitiveParams.kind
     */
    kind: {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatCapsuleParams capsule = 1;
         */
        value: SceneDatCapsuleParams;
        case: "capsule";
    } | {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatConicalFrustumParams conical_frustum = 2;
         */
        value: SceneDatConicalFrustumParams;
        case: "conicalFrustum";
    } | {
        /**
         * @generated from field: gravimera.scene.v1.SceneDatTorusParams torus = 3;
         */
        value: SceneDatTorusParams;
        case: "torus";
    } | {
        case: undefined;
        value?: undefined;
    };
};
/**
 * Describes the message gravimera.scene.v1.SceneDatPrimitiveParams.
 * Use `create(SceneDatPrimitiveParamsSchema)` to create a new message.
 */
export declare const SceneDatPrimitiveParamsSchema: GenMessage<SceneDatPrimitiveParams>;
/**
 * @generated from message gravimera.scene.v1.SceneDatCapsuleParams
 */
export type SceneDatCapsuleParams = Message<"gravimera.scene.v1.SceneDatCapsuleParams"> & {
    /**
     * @generated from field: float radius = 1;
     */
    radius: number;
    /**
     * @generated from field: float half_length = 2;
     */
    halfLength: number;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatCapsuleParams.
 * Use `create(SceneDatCapsuleParamsSchema)` to create a new message.
 */
export declare const SceneDatCapsuleParamsSchema: GenMessage<SceneDatCapsuleParams>;
/**
 * @generated from message gravimera.scene.v1.SceneDatConicalFrustumParams
 */
export type SceneDatConicalFrustumParams = Message<"gravimera.scene.v1.SceneDatConicalFrustumParams"> & {
    /**
     * @generated from field: float radius_top = 1;
     */
    radiusTop: number;
    /**
     * @generated from field: float radius_bottom = 2;
     */
    radiusBottom: number;
    /**
     * @generated from field: float height = 3;
     */
    height: number;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatConicalFrustumParams.
 * Use `create(SceneDatConicalFrustumParamsSchema)` to create a new message.
 */
export declare const SceneDatConicalFrustumParamsSchema: GenMessage<SceneDatConicalFrustumParams>;
/**
 * @generated from message gravimera.scene.v1.SceneDatTorusParams
 */
export type SceneDatTorusParams = Message<"gravimera.scene.v1.SceneDatTorusParams"> & {
    /**
     * @generated from field: float minor_radius = 1;
     */
    minorRadius: number;
    /**
     * @generated from field: float major_radius = 2;
     */
    majorRadius: number;
};
/**
 * Describes the message gravimera.scene.v1.SceneDatTorusParams.
 * Use `create(SceneDatTorusParamsSchema)` to create a new message.
 */
export declare const SceneDatTorusParamsSchema: GenMessage<SceneDatTorusParams>;
/**
 * @generated from enum gravimera.scene.v1.SceneDatMobilityMode
 */
export declare enum SceneDatMobilityMode {
    /**
     * @generated from enum value: GROUND = 0;
     */
    GROUND = 0,
    /**
     * @generated from enum value: AIR = 1;
     */
    AIR = 1
}
/**
 * Describes the enum gravimera.scene.v1.SceneDatMobilityMode.
 */
export declare const SceneDatMobilityModeSchema: GenEnum<SceneDatMobilityMode>;
/**
 * @generated from enum gravimera.scene.v1.SceneDatProjectileObstacleRule
 */
export declare enum SceneDatProjectileObstacleRule {
    /**
     * @generated from enum value: BULLETS_BLOCKERS = 0;
     */
    BULLETS_BLOCKERS = 0,
    /**
     * @generated from enum value: LASER_BLOCKERS = 1;
     */
    LASER_BLOCKERS = 1
}
/**
 * Describes the enum gravimera.scene.v1.SceneDatProjectileObstacleRule.
 */
export declare const SceneDatProjectileObstacleRuleSchema: GenEnum<SceneDatProjectileObstacleRule>;
/**
 * @generated from enum gravimera.scene.v1.SceneDatUnitAttackKind
 */
export declare enum SceneDatUnitAttackKind {
    /**
     * @generated from enum value: UNKNOWN = 0;
     */
    UNKNOWN = 0,
    /**
     * @generated from enum value: MELEE = 1;
     */
    MELEE = 1,
    /**
     * @generated from enum value: RANGED_PROJECTILE = 2;
     */
    RANGED_PROJECTILE = 2
}
/**
 * Describes the enum gravimera.scene.v1.SceneDatUnitAttackKind.
 */
export declare const SceneDatUnitAttackKindSchema: GenEnum<SceneDatUnitAttackKind>;
/**
 * @generated from enum gravimera.scene.v1.SceneDatPartAnimationFamily
 */
export declare enum SceneDatPartAnimationFamily {
    /**
     * @generated from enum value: BASE = 0;
     */
    BASE = 0,
    /**
     * @generated from enum value: OVERLAY = 1;
     */
    OVERLAY = 1
}
/**
 * Describes the enum gravimera.scene.v1.SceneDatPartAnimationFamily.
 */
export declare const SceneDatPartAnimationFamilySchema: GenEnum<SceneDatPartAnimationFamily>;
/**
 * @generated from enum gravimera.scene.v1.SceneDatPartAnimationDriver
 */
export declare enum SceneDatPartAnimationDriver {
    /**
     * @generated from enum value: ALWAYS = 0;
     */
    ALWAYS = 0,
    /**
     * @generated from enum value: MOVE_PHASE = 1;
     */
    MOVE_PHASE = 1,
    /**
     * @generated from enum value: MOVE_DISTANCE = 2;
     */
    MOVE_DISTANCE = 2,
    /**
     * @generated from enum value: ATTACK_TIME = 3;
     */
    ATTACK_TIME = 3,
    /**
     * @generated from enum value: ACTION_TIME = 4;
     */
    ACTION_TIME = 4
}
/**
 * Describes the enum gravimera.scene.v1.SceneDatPartAnimationDriver.
 */
export declare const SceneDatPartAnimationDriverSchema: GenEnum<SceneDatPartAnimationDriver>;
/**
 * @generated from enum gravimera.scene.v1.SceneDatPartAnimationSpinAxisSpace
 */
export declare enum SceneDatPartAnimationSpinAxisSpace {
    /**
     * @generated from enum value: JOIN = 0;
     */
    JOIN = 0,
    /**
     * @generated from enum value: CHILD_LOCAL = 1;
     */
    CHILD_LOCAL = 1
}
/**
 * Describes the enum gravimera.scene.v1.SceneDatPartAnimationSpinAxisSpace.
 */
export declare const SceneDatPartAnimationSpinAxisSpaceSchema: GenEnum<SceneDatPartAnimationSpinAxisSpace>;
/**
 * @generated from enum gravimera.scene.v1.SceneDatMeshKey
 */
export declare enum SceneDatMeshKey {
    /**
     * @generated from enum value: UNIT_CUBE = 0;
     */
    UNIT_CUBE = 0,
    /**
     * @generated from enum value: UNIT_CYLINDER = 1;
     */
    UNIT_CYLINDER = 1,
    /**
     * @generated from enum value: UNIT_CONE = 2;
     */
    UNIT_CONE = 2,
    /**
     * @generated from enum value: UNIT_SPHERE = 3;
     */
    UNIT_SPHERE = 3,
    /**
     * @generated from enum value: UNIT_PLANE = 4;
     */
    UNIT_PLANE = 4,
    /**
     * @generated from enum value: UNIT_CAPSULE = 5;
     */
    UNIT_CAPSULE = 5,
    /**
     * @generated from enum value: UNIT_CONICAL_FRUSTUM = 6;
     */
    UNIT_CONICAL_FRUSTUM = 6,
    /**
     * @generated from enum value: UNIT_TORUS = 7;
     */
    UNIT_TORUS = 7,
    /**
     * @generated from enum value: UNIT_TRIANGLE = 8;
     */
    UNIT_TRIANGLE = 8,
    /**
     * @generated from enum value: UNIT_TETRAHEDRON = 9;
     */
    UNIT_TETRAHEDRON = 9,
    /**
     * @generated from enum value: TREE_TRUNK = 10;
     */
    TREE_TRUNK = 10,
    /**
     * @generated from enum value: TREE_CONE = 11;
     */
    TREE_CONE = 11
}
/**
 * Describes the enum gravimera.scene.v1.SceneDatMeshKey.
 */
export declare const SceneDatMeshKeySchema: GenEnum<SceneDatMeshKey>;
