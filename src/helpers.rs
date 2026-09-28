use bevy::prelude::*;

/// Lifecycle marker for per-character morph state. Place on a `CharacterShape` entity.
///
/// Per-shape morphed-body deltas live on the shared [`CharacterTemplate`](crate::template::CharacterTemplate)
/// (baked once per template, sparse: a nose shape only touches nose verts).
/// Skeleton fitting, collider spawning, and bind-pose reset blend
/// `base + Σ weight × delta` on demand from those deltas plus the character's
/// own weights — no 150KB helper vec is stored per character, and no
/// background task runs per character.
///
/// Weights are free: the game decides the convention. The usual split is macro
/// shapes (baby, bodybuilder) with weights summing to 1, plus feature shapes
/// (long nose, big ears) with independent 0..1 weights added on top.
/// humentity never normalizes; any auto-normalize lives in game code, not here.
///
/// Removing this component tears the character's render/physics state back down to a
/// bare state blob: the skeleton, `CharacterSkeleton`/`SkeletonsReady`, per-part mesh
/// handles, and (avian) colliders, joints, `NeedsColliders` are all cleaned up.
/// humentity registers an `On<Remove, HelperVertexPositions>` observer for this. Register
/// your own observer on the same trigger to clean up additional per-character data that
/// humentity can't know about generically, such as material handles:
///
/// ```ignore
/// app.add_observer(
///     |trigger: On<Remove, humentity::prelude::HelperVertexPositions>,
///      mut commands: Commands| {
///         // e.g. commands.entity(trigger.entity()).remove::<MeshMaterial3d>()
///     },
/// );
/// ```
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct HelperVertexPositions;
