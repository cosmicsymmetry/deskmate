#pragma once

/* The TinyCBOR-coupled half of core/scene_decode.c.
 *
 * scene_model.h declares scene_decode(), which takes a standalone buffer,
 * and deliberately mentions no CBOR type at all so scene_model.c, the
 * simulator and the host tests can build scenes directly. This header is
 * the exception: it exists for the one caller that already holds an open
 * CborValue -- core/protocol_message.c decoding a PushScene, whose scene
 * arrives as a NESTED map under key 2 rather than as a payload of its own.
 *
 * Include this only where TinyCBOR is already in scope. Nothing else in
 * core/ needs it. */

#include "cbor.h"

#include "core/scene_model.h"

/* Decodes one scene from a CBOR map the caller has already reached but not
 * entered, advancing `value` past that map on success.
 *
 * This is the body of scene_decode(); that function is this plus opening
 * and whole-payload-validating a standalone buffer. Both go through here,
 * so there is one scene-map decoder, not two that must agree.
 *
 * WHAT THE CALLER MUST HAVE ESTABLISHED. This function does not validate
 * the encoding -- it reads a map whose bytes some enclosing cbor_value_
 * validate() has already passed. Callers must have applied, at their root:
 *
 *     CborValidateCanonicalFormat | CborValidateMapKeysAreUnique |
 *     CborValidateUtf8 | CborValidateNoUndefined | CborValidateNoTags
 *
 * core/protocol_message.c's open_payload_map() applies exactly that set
 * (plus CborValidateCompleteData) to the whole PushScene payload at the
 * root, so a scene nested inside it inherits every one of them. The one
 * flag a nested scene does NOT inherit is CborValidateCompleteData, and it
 * should not: trailing bytes are the enclosing payload's business, and the
 * enclosing payload is where that flag is checked.
 *
 * The result contract is scene_decode()'s, unchanged and repeated here
 * because this is the entry point protocol_message.c actually calls:
 *
 *   On SCENE_MODEL_OK, `*out` has ALREADY PASSED scene_model_validate().
 *   On ANY non-OK result, `out->node_count` is 0 -- the scene is refused
 *   whole, never half-decoded. (Unlike scene_decode(), this function does
 *   NOT zero the whole of `*out`: sizeof(scene_t) is ~6 KB and a caller
 *   that already cleared its message struct should not pay for a second
 *   pass. It clears node_count on entry, which is what the contract above
 *   is actually about; the node array below that count is meaningless
 *   either way.)
 *
 * See scene_model.h's scene_decode() comment for the full result mapping,
 * and the top of scene_decode.c for the wire shape. */
scene_model_result_t scene_decode_map(CborValue *value, scene_t *out);
