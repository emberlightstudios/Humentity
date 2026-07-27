use crate::{
    loaders::{MhcloAsset, MhcloVertexMap},
    mesh_ops::{
        fix_normals, fix_normals_multiple, generate_mhid_lookup, generate_vertex_map,
        get_uv_coords, get_vertex_normals, get_vertex_positions, get_vertex_tangents,
    },
    morphs::adjust_helpers_to_morphs,
    prelude::*,
    rigs::{set_asset_rig_arrays, RigSpec},
    template::TemplateOverride,
};
use ahash::{AHashMap, AHashSet};
use bevy::mesh::morph::MorphAttributes;
use bevy::{asset::RenderAssetUsages, prelude::*};
use std::sync::{Arc, RwLock};

/// Collection of parts that should be stitched together.  This will
/// spawn siblings for each part then despawn this entity.
#[derive(Component, Clone, Deref, DerefMut, Eq, PartialEq, Hash, Debug)]
pub struct StitchedParts(pub Vec<StitchedPart>);

#[derive(Clone, Eq, PartialEq, Hash, Debug)]
pub struct StitchedPart {
    pub(crate) part: Handle<MhcloAsset>,
    pub(crate) template_override: Option<TemplateOverride>,
    pub(crate) lod: usize,
}

impl From<Handle<MhcloAsset>> for StitchedPart {
    fn from(part: Handle<MhcloAsset>) -> Self {
        Self {
            part,
            template_override: None,
            lod: 0,
        }
    }
}

impl StitchedPart {
    pub fn with_template_override(mut self, template: Handle<CharacterTemplate>) -> Self {
        self.template_override = Some(TemplateOverride(template));
        self
    }

    pub const fn with_lod(mut self, lod: usize) -> Self {
        self.lod = lod;
        self
    }
}

pub fn shape_mesh_from_helpers_mhclo(
    mesh: &Mesh,
    mhclo: &MhcloAsset,
    helpers: &[Vec3],
    mhid_lookup: &[u16],
    bevy_vertex_map: &AHashMap<u16, Vec<u16>>,
) -> Mesh {
    let mut vertices = get_vertex_positions(mesh);
    for (vert, mh_asset_vertex) in mhid_lookup.iter().enumerate() {
        let hm = &mhclo.helper_map[*mh_asset_vertex as usize];
        match hm {
            MhcloVertexMap::SingleVertex(i) => {
                vertices[vert] = helpers[*i as usize];
            }
            MhcloVertexMap::Triangle {
                helper_verts,
                helper_weights,
                helper_offset,
            } => {
                let mut position = Vec3::ZERO;
                for i in 0..3 {
                    let mh_vert = helper_verts[i];
                    let wt = helper_weights[i];
                    if let Some(mh_helper_position) = helpers.get(mh_vert as usize) {
                        position += *mh_helper_position * wt;
                    }
                }
                let offset = mhclo.get_offset_scale_mhclo(helpers) * helper_offset;
                vertices[vert] = position + offset;
            }
        }
    }

    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vertices)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, get_uv_coords(mesh))
    .with_inserted_indices(mesh.indices().unwrap().clone())
    .with_computed_area_weighted_normals()
    .with_generated_tangents()
    .expect("Failed to generate tangents?");
    fix_normals(&mut mesh, bevy_vertex_map);
    mesh
}

pub(crate) fn build_final_mesh_mhclo(
    mhclo: &MhcloAsset,
    input_mesh: &Mesh,
    mesh_verts: &ObjVertsAsset,
    template: &CharacterTemplate,
    mh_morphs: Arc<RwLock<AHashMap<&'static str, TargetAsset>>>,
    basemesh: Arc<Vec<Vec3>>,
    _rig_spec: &RigSpec,
    lod_bone_names: &[&'static str],
    lod_weights: &AHashMap<&'static str, AHashMap<u16, f32>>,
) -> (Mesh, Vec<String>) {
    let vertices = get_vertex_positions(input_mesh);
    let vertex_map = generate_vertex_map(&mesh_verts.vertices, &vertices);
    let mhid_lookup = generate_mhid_lookup(&vertex_map);

    let mut input_mesh =
        shape_mesh_from_helpers_mhclo(input_mesh, mhclo, &basemesh, &mhid_lookup, &vertex_map);

    let mut meshes = vec![];
    for shape in template.shapes.iter() {
        let helpers = adjust_helpers_to_morphs(&shape.morphs, &mh_morphs, &basemesh)
            .unwrap_or_else(|e| panic!("{}", e));
        let mesh =
            shape_mesh_from_helpers_mhclo(&input_mesh, mhclo, &helpers, &mhid_lookup, &vertex_map);
        meshes.push(mesh);
    }

    let mut morph_names = vec![];
    if template.shapes.len() > 1 {
        let base_positions = get_vertex_positions(&input_mesh);
        let base_normals = get_vertex_normals(&input_mesh);
        let base_tangents = get_vertex_tangents(&input_mesh).expect("Failed to get tangents");
        let mut morphs = vec![];

        for (is, shape_mesh) in meshes.iter().enumerate() {
            let mut morph = Vec::<MorphAttributes>::new();
            let shape_positions = get_vertex_positions(shape_mesh);
            let shape_normals = get_vertex_normals(shape_mesh);
            let shape_tangents =
                get_vertex_tangents(shape_mesh).expect("Shape meshes should always have tangents");

            for vtx in 0..base_positions.len() {
                morph.push(MorphAttributes::from([
                    shape_positions[vtx] - base_positions[vtx],
                    shape_normals[vtx] - base_normals[vtx],
                    shape_tangents[vtx] - base_tangents[vtx],
                ]));
            }

            morph_names.push(template.shapes[is].name.to_string());
            morphs.push(morph.into_iter());
        }
        let morph_attributes: Vec<MorphAttributes> = morphs.into_iter().flatten().collect();
        input_mesh.set_morph_targets(morph_attributes);
    } else if template.shapes.len() == 1 {
        input_mesh = meshes.into_iter().next().unwrap();
    }

    set_asset_rig_arrays(
        &mut input_mesh,
        &mhid_lookup,
        &mhclo.helper_map,
        lod_bone_names,
        lod_weights,
    );

    (input_mesh, morph_names)
}

pub(crate) fn build_final_meshes_mhclo(
    mhclos: &[MhcloAsset],
    input_meshes: &mut [Mesh],
    mesh_verts: &[ObjVertsAsset],
    templates: &[CharacterTemplate],
    mh_morphs: Arc<RwLock<AHashMap<&'static str, TargetAsset>>>,
    basemesh: Arc<Vec<Vec3>>,
    _rig_spec: &RigSpec,
    lod_bone_names: &[&'static str],
    lod_weights: &AHashMap<&'static str, AHashMap<u16, f32>>,
) -> (Vec<Mesh>, Vec<Vec<String>>) {
    let mut mhid_lookup = vec![];
    let mut vertex_map = vec![];

    let n_meshes = input_meshes.len();
    for i in 0..n_meshes {
        let mesh = &input_meshes[i];
        let vertices = get_vertex_positions(mesh);
        vertex_map.push(generate_vertex_map(&mesh_verts[i].vertices, &vertices));
        mhid_lookup.push(generate_mhid_lookup(&vertex_map[i]));
        input_meshes[i] = shape_mesh_from_helpers_mhclo(
            mesh,
            &mhclos[i],
            &basemesh,
            &mhid_lookup[i],
            &vertex_map[i],
        );
    }

    let shapes: AHashSet<_> = templates
        .iter()
        .flat_map(|p| &p.shapes)
        .map(|s| s.name)
        .collect();

    let mut shape_meshes = AHashMap::default();
    for &shape in shapes.iter() {
        let mut meshes = vec![];
        for (i_mesh, mesh) in input_meshes.iter().enumerate() {
            let template = &templates[i_mesh];
            let mut matched = false;
            for mesh_shape in template.shapes.iter() {
                if shape == mesh_shape.name {
                    let helpers =
                        adjust_helpers_to_morphs(&mesh_shape.morphs, &mh_morphs, &basemesh)
                            .unwrap_or_else(|e| panic!("{}", e));
                    meshes.push(Some(shape_mesh_from_helpers_mhclo(
                        mesh,
                        &mhclos[i_mesh],
                        &helpers,
                        &mhid_lookup[i_mesh],
                        &vertex_map[i_mesh],
                    )));
                    matched = true;
                }
            }
            if !matched {
                meshes.push(None);
            }
        }

        let mut tmp_mesh_vec = meshes
            .iter_mut()
            .filter_map(|m| m.as_mut())
            .collect::<Vec<_>>();
        fix_normals_multiple(&mut tmp_mesh_vec);
        shape_meshes.insert(shape, meshes);
    }

    let mut morph_names = vec![];

    for i_mesh in 0..n_meshes {
        let mut names = vec![];
        let template = &templates[i_mesh];

        if template.shapes.len() > 1 {
            let base_positions = get_vertex_positions(&input_meshes[i_mesh]);
            let base_normals = get_vertex_normals(&input_meshes[i_mesh]);
            let base_tangents =
                get_vertex_tangents(&input_meshes[i_mesh]).expect("Failed to get tangents");
            let mut morph_attrs = vec![];

            for shape in template.shapes.iter() {
                let Some(shape_mesh) = &shape_meshes[shape.name][i_mesh] else {
                    continue;
                };
                let mut morph = Vec::<MorphAttributes>::new();

                let shape_positions = get_vertex_positions(shape_mesh);
                let shape_normals = get_vertex_normals(shape_mesh);
                let shape_tangents = get_vertex_tangents(shape_mesh)
                    .expect("Shape meshes should always have tangents");

                for vtx in 0..base_positions.len() {
                    morph.push(MorphAttributes::from([
                        shape_positions[vtx] - base_positions[vtx],
                        shape_normals[vtx] - base_normals[vtx],
                        shape_tangents[vtx] - base_tangents[vtx],
                    ]));
                }

                names.push(shape.name.to_string());
                morph_attrs.push(morph.into_iter());
            }
            let morph_attributes: Vec<MorphAttributes> =
                morph_attrs.into_iter().flatten().collect();
            input_meshes[i_mesh].set_morph_targets(morph_attributes);
        } else if template.shapes.len() == 1 {
            let Some(shape_mesh) = &shape_meshes[template.shapes[0].name][i_mesh] else {
                continue;
            };
            input_meshes[i_mesh] = shape_mesh.clone();
        }
        morph_names.push(names);

        set_asset_rig_arrays(
            &mut input_meshes[i_mesh],
            &mhid_lookup[i_mesh],
            &mhclos[i_mesh].helper_map,
            lod_bone_names,
            lod_weights,
        );
    }

    (input_meshes.to_vec(), morph_names)
}
