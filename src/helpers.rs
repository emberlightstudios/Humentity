use std::iter::FromIterator;

use bevy::{prelude::*, tasks::AsyncComputeTaskPool};
use crossbeam_channel::{Receiver, Sender};

use crate::{
    loaders::{CharacterShapeAsset, TargetAsset},
    morphs::{adjust_helpers_with_targets, MakeHumanMorphs, MorphTargets},
    prelude::{BaseMesh, CharacterShape},
    template::CharacterTemplate,
};

/// A cache for morph-deformed base-mesh vertex positions for a single character.
///
/// Insert `HelperVertexPositions::default()` (empty) on a `CharacterShape` entity to
/// request background computation. A system detects the `Added` event, spawns a
/// background task, and fills the vec once complete. Both skeleton fitting and
/// collider spawning read from this.
///
/// To re-fit (e.g. after morph changes), remove and re-insert
/// `HelperVertexPositions::default()`.
///
/// Removing this component tears the character's render/physics state back down to a
/// bare state blob: the skeleton, `CharacterSkeleton`/`SkeletonsReady`, per-part mesh
/// handles, and (avian) colliders, joints, `NeedsColliders` are all cleaned up.
/// humentity registers an `On<Remove, HelperVertexPositions>` observer for this. Register
/// your own observer on the same trigger to clean up additional per-character data that
/// humentity can't know about generically, such as material handles:
///
/// ```
/// app.add_observer(
///     |trigger: On<Remove, humentity::prelude::HelperVertexPositions>,
///      mut commands: Commands| {
///         // e.g. commands.entity(trigger.entity()).remove::<MeshMaterial3d>()
///     },
/// );
/// ```
#[derive(Component, Debug, Clone, Default)]
pub struct HelperVertexPositions(pub Vec<Vec3>);

impl FromIterator<Vec3> for HelperVertexPositions {
    fn from_iter<T: IntoIterator<Item = Vec3>>(iter: T) -> Self {
        HelperVertexPositions(iter.into_iter().collect())
    }
}

/// Channel message sent from a background task when helpers are ready.
struct HelperResult {
    character: Entity,
    helpers: Vec<Vec3>,
}

/// Tracks in-flight background helper computations.
#[derive(Resource, Default)]
pub(crate) struct HelperComputeJobs {
    sender: Option<Sender<HelperResult>>,
    receiver: Option<Receiver<HelperResult>>,
}

impl HelperComputeJobs {
    fn ensure_channels(&mut self) {
        if self.sender.is_none() {
            let (tx, rx) = crossbeam_channel::unbounded();
            self.sender = Some(tx);
            self.receiver = Some(rx);
        }
    }
}

/// Pure computation: resolve template morph weights into per-vertex helpers.
/// Runs on a background thread with no ECS access.
fn compute_helpers_from_data(
    morph_values: &MorphTargets,
    template_shapes: &[crate::template::CharacterMorphShape],
    targets: &ahash::AHashMap<&'static str, TargetAsset>,
    basemesh_vertices: &[Vec3],
) -> Result<Vec<Vec3>, crate::morphs::MorphError> {
    let mut mh_morph_values = MorphTargets::default();
    for shape in template_shapes.iter() {
        let Some(weight) = morph_values.get(shape.name) else {
            continue;
        };
        for (&k, v) in shape.morphs.iter() {
            *mh_morph_values.entry(k).or_insert(0.) += *v * weight;
        }
    }
    adjust_helpers_with_targets(&mh_morph_values, targets, basemesh_vertices)
}

/// Watches for newly-added `HelperVertexPositions` components with empty vecs and spawns
/// background tasks to compute them.
pub(crate) fn submit_helper_computations(
    characters: Query<
        (Entity, &CharacterShape, &HelperVertexPositions),
        Added<HelperVertexPositions>,
    >,
    shape_assets: Res<Assets<CharacterShapeAsset>>,
    templates: Res<Assets<CharacterTemplate>>,
    basemesh: Res<BaseMesh>,
    morphs: Res<MakeHumanMorphs>,
    mut jobs: ResMut<HelperComputeJobs>,
) {
    jobs.ensure_channels();
    let tx = jobs.sender.as_ref().unwrap().clone();

    // Clone the Arc so background tasks can read without locking the resource.
    let morphs_arc = morphs.targets.clone();

    let pool = AsyncComputeTaskPool::get();

    for (entity, character_shape, helpers) in &characters {
        if !helpers.0.is_empty() {
            continue;
        }

        let Some(asset) = shape_assets.get(&character_shape.0) else {
            continue;
        };
        let Some(template) = templates.get(&asset.template) else {
            continue;
        };

        let morph_values = asset.template_morph_targets.clone();
        let template_shapes = template.shapes.clone();
        let basemesh_vertices = basemesh.vertices.clone();
        let morphs_ref = morphs_arc.clone();
        let tx = tx.clone();

        pool.spawn(async move {
            let result = compute_helpers_from_data(
                &morph_values,
                &template_shapes,
                &morphs_ref.read().unwrap(),
                &basemesh_vertices,
            );
            if let Ok(helpers) = result {
                let _ = tx.send(HelperResult {
                    character: entity,
                    helpers,
                });
            }
        })
        .detach();
    }
}

/// Polls for completed background tasks and fills in the `HelperVertexPositions` component
/// on the corresponding `CharacterShape` entities.
pub(crate) fn collect_helper_computations(
    mut helpers_query: Query<&mut HelperVertexPositions>,
    jobs: ResMut<HelperComputeJobs>,
) {
    let Some(rx) = jobs.receiver.as_ref() else {
        return;
    };
    for result in rx.try_iter() {
        if let Ok(mut h) = helpers_query.get_mut(result.character) {
            h.0 = result.helpers;
        }
    }
}
