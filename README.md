# Humentity (MakeHuman inside Bevy)

![Alt text](https://i.imghippo.com/files/eVWiu1727317384.png)

## Current features
- Asset loaders for makehuman data files.
- Template-based morphable humanoid meshes
- Auto-rigging
- Animation retargeting
- LODs
- Stitched mesh fragments 
- Normal map smoothing
- Customizable equipment and body parts
- Custom morph targets

## Future Plans
- Working ragdolls 
- Baked posed meshes
- Root motion support

## Auto-build humanoids
All needed types should be exported via the crate prelude module.  The steps to create humanoid characters are as follows.
1. Add the HumentityPlugin.
2. Add the resources pointing to the necessary asset paths, as in the examples.
3. Create or load a humanoid template with some shapes.
4. Create or load a CharacterShapeAsset with weights for those shapes.
4. Create an entity with a CharacterShape component. 
5. Add children with CharacterPart components.

### Notes
You can find working examples in the examples folder.

Character templates are defined with a vec of specified shapes. 
To build a mesh use the MeshBuildJob struct.
The crate will build and cache a Handle\<Mesh\> for you which will have the template shapes as morph targets on the mesh.

Characters are entities with a CharacterShape component.
Mhclo files define the meshes that make up your character.
You can apply template shapes to them as morph targets using the MeshBuildJob struct.
This will build the mesh and store it in a cache for you, which is exposed as a resource.

Currently rigs are added to characters automatically, as defined on the template.
For animation retargeting author clips on the basemesh with no shapekeys.
By default translation tracks are dropped on import.
Support for translation tracks is very experimental still and requires an animation post-processing system so there is an extra cost.

Custom assets can be built inside Blender with MPFB to export .mhclo/.obj files, custom .target files too.
