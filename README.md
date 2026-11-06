# quby seemdissolver

Download the executable from [Releases](https://github.com/wasnas/quby-seemdissolver/releases/latest). Open quby-seemdissolver.exe on Windows x64. A sandstone demo loads immediately.

1. Import a PNG, JPEG, WebP, TIFF or BMP, or drop it onto the window.
2. Choose **3 × 3 repeat** to inspect the joins. Increase seam overlap if the source has uneven edges. The overlap is cropped out, so a stronger overlap yields a smaller tile.
3. Set relief strength, roughness and metallic. Use **Invert height** when bright areas should recede. Choose DirectX normals when your renderer expects −Y. Material previews update automatically as controls change.
4. Inspect each map or rotate the light on the sphere. **Export PBR set** writes a fresh numbered folder containing seven PNGs.

Use sRGB for base color. Use non-color/linear data for normal, roughness, height, AO, metallic and ORM. The normal map uses tangent space. Connect it through a normal-map node. Packed ORM uses red for occlusion, green for roughness, and blue for metallic.

quby seemdissolver cuts low-error paths through overlapping borders and computes maps with wrapped sampling. The input photo supplies the color and brightness-based height estimate. Baked highlights and shadows can carry through. Metallic is an artist-set value. Roughness and occlusion are estimates. An evenly lit, front-facing photo works best.

The longest input side scales to 4096 px for processing. Input limit is 64 megapixels, minimum working dimension 16 px. Map previews may be downsampled. Exports use the full generated tile dimensions.

## Build from source

Windows x64, Rust stable with the MSVC toolchain and Visual Studio C++ build tools. Run `cargo build --release --locked` in the source folder. Run checks with `cargo test --release --locked`.

## Seam controls

Choose Adaptive cut for detailed surfaces, Straight cut to choose the join position, or Soft blend for gradients. Horizontal and vertical overlap set how much each pair of edges shares. Feather radius softens either cut; it is limited by the available overlap. Click Apply changes, then inspect the 3 × 3 repeat. Show tile boundaries marks the joins between repeats in the preview only.

## Preview navigation

Hover over the single tile, 3 × 3 repeat, or sphere and scroll to zoom between 50% and 800% of fit. Drag to pan when zoomed in. Double-click or select **Fit** to reset that view. Each view remembers its own framing. Zoom changes the preview only.

## License

The application code and bundled logo are licensed under MIT. Barlow and Barlow Semi Condensed are distributed under the SIL Open Font License. Their notices are included in `assets/fonts`.


## Live controls

Material settings update the texture and PBR previews automatically after a short pause. The app waits briefly while a slider moves so it can build the latest setting without making the controls feel sluggish.

## Recadrage

Ouvre l’onglet **Recadrage** pour déplacer ou redimensionner la zone de sélection. Choisis une proportion libre, carrée, 4:3, 16:9 ou 2:3. Clique **Recadrer et appliquer** pour recalculer automatiquement les textures et cartes PBR.



## Interface language

Use the compact EN/FR button in the header to switch languages. English is selected at launch.
