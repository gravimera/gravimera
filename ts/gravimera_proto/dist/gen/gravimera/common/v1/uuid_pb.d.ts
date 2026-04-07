import type { GenFile, GenMessage } from "@bufbuild/protobuf/codegenv2";
import type { Message } from "@bufbuild/protobuf";
/**
 * Describes the file gravimera/common/v1/uuid.proto.
 */
export declare const file_gravimera_common_v1_uuid: GenFile;
/**
 * A UUID stored as two u64 words (big-endian when reassembled into u128).
 *
 * @generated from message gravimera.common.v1.Uuid128
 */
export type Uuid128 = Message<"gravimera.common.v1.Uuid128"> & {
    /**
     * @generated from field: fixed64 hi = 1;
     */
    hi: bigint;
    /**
     * @generated from field: fixed64 lo = 2;
     */
    lo: bigint;
};
/**
 * Describes the message gravimera.common.v1.Uuid128.
 * Use `create(Uuid128Schema)` to create a new message.
 */
export declare const Uuid128Schema: GenMessage<Uuid128>;
