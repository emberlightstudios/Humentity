# Humentity (MakeHuman inside Bevy)

![Alt text](https://i.imghippo.com/files/eVWiu1727317384.png)

## Current features
- Shape-able humans
- Auto-rigging
- Animation retargeting
- LOD
- Customizable equipment and body parts
- Customizable morph targets

## Future Plans
- Better shaders with overlay textures
- Working ragdolls when physics support matures

## How It Works
All needed types should be exported via the crate prelude module.  The steps to create humanoid characters are as follows.
1. Setup/add the HumentityPlugin.
2. Create a humanoid prefab with some shapes.
3. Create an entity with a CharacterShapeConfig component.
4. Add children with CharacterPart components.

### Notes
You can find working examples in the examples folder.

The prefab you build contains a rig spec for animation.
You can provide .glb files with animation clips.
It is assumed the clips will be authored for the character in the base mesh with no shape keys applied, so remove all shapekeys in blender before making clips.
Clips should be retargetable to any character using the same prefab.
By default translation tracks are dropped.
Support for translation tracks is very experimental still and requires an animation post-processing system so there is an extra cost.
The .glb files you supply are pre-processed outside the Bevy Asset system, so the paths you supply should be readable from your cwd, not relative to the assets folder in the usual way.
This also applies to .target files (custom morphs) which don't interface with the asset system at all.

Custom assets can be built inside Blender with MPFB to export .mhclo/.obj files, custom .target files too.
There are special path types to handle loading assets from different bevy asset sources. See the shared module in examples.
You must set up paths for the relevant CharacterPart types, body parts, equipment, body meshes, skin_textures, targets, etc..
Texture maps for CharacterPart meshes must be in subfolders beside your asset .mhclo/.obj files, ./albedo, ./normal, ./ao, ./roughness_metallic, etc..

Regarding asset loading and unloading, for textures this crate only stores the paths in the registry.
CharacterPart provides an API for quick retrieval of a Handle<Image> but it is not cached anywhere inside the plugin, so automatic texture unloading should just work when you remove the last assets (e.g. material) referencing the image.
The meshes are a different story.
Meshes for the CharacterParts will be built automatically in a background thread on any (not disabled) entity satisfying (With<CharacterPart>, Without<Mesh3d>).
Mesh handles **are** cached inside the CharacterAssetRegistry (per prefab) because mesh construction is a slower process requiring a bit of calculation, so to completely unload the mesh assets the cached handles must be manually removed.
There is a helper fn on CharacterPart for this purpose.
