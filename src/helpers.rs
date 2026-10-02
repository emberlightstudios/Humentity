use bevy::prelude::*;

/// Entity event: tear a character's render/physics state back down to a bare
/// state blob. Fire to retire a character (`commands.trigger(TeardownCharacter(character))`):
/// the skeleton, `CharacterSkeleton`/`SkeletonReady`, per-part mesh handles,
/// and (avian) colliders, joints, `NeedsColliders` are all cleaned up.
/// humentity observes this event for teardown. Observe the same event to clean
/// up additional per-character data that humentity can't know about
/// generically, such as material handles:
///
/// ```ignore
/// app.add_observer(
///     |trigger: On<TeardownCharacter>| {
///         // e.g. commands.entity(trigger.entity()).remove::<MeshMaterial3d>()
///     },
/// );
/// ```
#[derive(EntityEvent, Debug, Clone, Copy)]
pub struct TeardownCharacter(pub Entity);

/// Entity event: strip a fitted character back to bare state so the next fit
/// pass rebuilds it from current morph weights (e.g. after morph changes).
/// Same teardown as [`TeardownCharacter`], plus re-inserts `FitSkeleton` so
/// `spawn_rig_skeleton`/`fit_skeleton_to_shape` refit on the next frames.
#[derive(EntityEvent, Debug, Clone, Copy)]
pub struct RefitCharacter(pub Entity);
