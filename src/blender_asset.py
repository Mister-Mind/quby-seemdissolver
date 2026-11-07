"""Build a self-contained Blender material library from Quby's exported maps."""

import math
from pathlib import Path
import sys

import bpy


MAPS = ("Base Color", "Roughness", "Metallic", "Height")


class Graph:
    def __init__(self, tree):
        self.tree = tree

    def node(self, kind, label=""):
        node = self.tree.nodes.new(kind)
        node.label = label
        return node

    def connect(self, value, socket):
        if isinstance(value, bpy.types.NodeSocket):
            self.tree.links.new(value, socket)
        else:
            socket.default_value = value

    def math(self, operation, a, b=None):
        node = self.node("ShaderNodeMath")
        node.operation = operation
        self.connect(a, node.inputs[0])
        if b is not None:
            self.connect(b, node.inputs[1])
        return node.outputs[0]

    def vector(self, operation, a, b=None):
        node = self.node("ShaderNodeVectorMath")
        node.operation = operation
        self.connect(a, node.inputs[0])
        if b is not None:
            self.connect(b, node.inputs["Scale"] if operation == "SCALE" else node.inputs[1])
        return node.outputs["Vector"]

    def xy(self, x, y):
        node = self.node("ShaderNodeCombineXYZ")
        self.connect(x, node.inputs["X"])
        self.connect(y, node.inputs["Y"])
        return node.outputs[0]

    def split(self, vector):
        node = self.node("ShaderNodeSeparateXYZ")
        self.connect(vector, node.inputs[0])
        return tuple(node.outputs[key] for key in "XYZ")

    def mix(self, factor, a, b, color=False):
        node = self.node("ShaderNodeMix")
        node.data_type = "RGBA" if color else "FLOAT"
        index_a, index_b, result = (6, 7, 2) if color else (2, 3, 0)
        self.connect(factor, node.inputs[0])
        self.connect(a, node.inputs[index_a])
        self.connect(b, node.inputs[index_b])
        return node.outputs[result]

    def layout(self):
        parents = {}
        for link in self.tree.links:
            parents.setdefault(link.to_node, []).append(link.from_node)
        depths = {}
        rows = {}

        def depth(node):
            if node not in depths:
                depths[node] = 1 + max((depth(parent) for parent in parents.get(node, [])), default=-1)
            return depths[node]

        for node in self.tree.nodes:
            column = depth(node)
            row = rows.get(column, 0)
            node.location = (column * 235, -row * 315)
            rows[column] = row + 1


def input_socket(tree, name, default, minimum, maximum, description):
    socket = tree.interface.new_socket(
        name, in_out="INPUT", socket_type="NodeSocketFloat", description=description
    )
    socket.default_value = default
    socket.min_value = minimum
    socket.max_value = maximum


def tile_sampler(images):
    tree = bpy.data.node_groups.new("Quby · Random Tile Sampler", "ShaderNodeTree")
    tree.interface.new_socket("Coordinates", in_out="INPUT", socket_type="NodeSocketVector")
    input_socket(tree, "Random Rotation", 1.0, 0.0, 1.0, "Maximum random rotation")
    for name in MAPS:
        tree.interface.new_socket(
            name, in_out="OUTPUT",
            socket_type="NodeSocketColor" if name == "Base Color" else "NodeSocketFloat",
        )
    graph = Graph(tree)
    inputs = graph.node("NodeGroupInput").outputs
    outputs = graph.node("NodeGroupOutput").inputs
    coords = inputs["Coordinates"]
    cell = graph.vector("FLOOR", coords)
    fraction = graph.vector("FRACTION", coords)
    ramp = graph.node("ShaderNodeMapRange", "Smooth tile boundaries")
    ramp.data_type = "FLOAT_VECTOR"
    ramp.interpolation_type = "SMOOTHERSTEP"
    graph.connect(fraction, ramp.inputs["Vector"])
    wx, wy, _ = graph.split(ramp.outputs["Vector"])
    samples = []
    for x, y in ((0, 0), (1, 0), (0, 1), (1, 1)):
        corner = graph.vector("ADD", cell, (x, y, 0))
        random = graph.node("ShaderNodeTexWhiteNoise", "Stable random angle per tile")
        graph.connect(corner, random.inputs["Vector"])
        angle = graph.math("MULTIPLY", graph.math("SUBTRACT", random.outputs["Value"], 0.5), math.tau)
        angle = graph.math("MULTIPLY", angle, inputs["Random Rotation"])
        rotate = graph.node("ShaderNodeVectorRotate", "Rotate all maps together")
        rotate.rotation_type = "Z_AXIS"
        graph.connect(graph.vector("SUBTRACT", coords, corner), rotate.inputs["Vector"])
        graph.connect(angle, rotate.inputs["Angle"])
        sample = {}
        for name, image in images.items():
            texture = graph.node("ShaderNodeTexImage", name)
            texture.image = image
            texture.extension = "REPEAT"
            graph.connect(rotate.outputs["Vector"], texture.inputs["Vector"])
            sample[name] = texture.outputs["Color"]
        samples.append(sample)
    for name in MAPS:
        color = name == "Base Color"
        bottom = graph.mix(wx, samples[0][name], samples[1][name], color)
        top = graph.mix(wx, samples[2][name], samples[3][name], color)
        graph.connect(graph.mix(wy, bottom, top, color), outputs[name])
    graph.layout()
    return tree


def material_group(images, mode):
    tree = bpy.data.node_groups.new("Quby Material", "ShaderNodeTree")
    for name, default, minimum, maximum, description in (
        ("Texture Scale", 1.0, 0.01, 100.0, "Number of texture repeats"),
        ("Random Rotation", 1.0, 0.0, 1.0, "Set to zero for regular repetition"),
        ("Bump Strength", 1.0, 0.0, 1.0, "Intensity of the surface relief"),
        ("Bump Size", 0.03, 0.0, 10.0, "Height scale of the surface relief"),
        ("Roughness Multiplier", 1.0, 0.0, 4.0, "Scale the roughness map"),
        ("Metallic Multiplier", 1.0, 0.0, 4.0, "Scale the metallic map"),
    ):
        input_socket(tree, name, default, minimum, maximum, description)
    tree.interface.new_socket("Shader", in_out="OUTPUT", socket_type="NodeSocketShader")
    graph = Graph(tree)
    inputs = graph.node("NodeGroupInput").outputs
    outputs = graph.node("NodeGroupOutput").inputs
    coords = graph.node("ShaderNodeTexCoord")
    uv = mode == "0"
    source = coords.outputs["UV" if uv else "Generated"]
    x, y, z = graph.split(graph.vector("SCALE", source, inputs["Texture Scale"]))
    planes = ((x, y),) if uv else ((y, z), (x, z), (x, y))
    sampler = tile_sampler(images)
    samples = []
    for (u, v), label in zip(planes, ("UV",) if uv else ("YZ", "XZ", "XY")):
        node = graph.node("ShaderNodeGroup", label)
        node.node_tree = sampler
        graph.connect(graph.xy(u, v), node.inputs["Coordinates"])
        graph.connect(inputs["Random Rotation"], node.inputs["Random Rotation"])
        samples.append(node.outputs)

    values = dict((name, samples[0][name]) for name in MAPS)
    if not uv:
        geometry = graph.node("ShaderNodeNewGeometry")
        normal = graph.node("ShaderNodeVectorTransform")
        normal.vector_type = "NORMAL"
        normal.convert_from = "WORLD"
        normal.convert_to = "OBJECT"
        graph.connect(geometry.outputs["True Normal"], normal.inputs["Vector"])
        nx, ny, nz = graph.split(graph.vector("ABSOLUTE", normal.outputs["Vector"]))
        if mode == "1":
            # Pick the strongest axis, with deterministic ties.
            xy = graph.math("GREATER_THAN", ny, nx)
            z_axis = graph.math("GREATER_THAN", nz, graph.math("MAXIMUM", nx, ny))
        else:
            nx, ny, nz = (graph.math("POWER", component, 4.0) for component in (nx, ny, nz))
            subtotal = graph.math("ADD", nx, ny)
            xy = graph.math("DIVIDE", ny, subtotal)
            z_axis = graph.math("DIVIDE", nz, graph.math("ADD", subtotal, nz))
        for name in MAPS:
            color = name == "Base Color"
            sides = graph.mix(xy, samples[0][name], samples[1][name], color)
            values[name] = graph.mix(z_axis, sides, samples[2][name], color)

    shader = graph.node("ShaderNodeBsdfPrincipled")
    graph.connect(values["Base Color"], shader.inputs["Base Color"])
    for name in ("Roughness", "Metallic"):
        value = graph.math("MULTIPLY", values[name], inputs[f"{name} Multiplier"])
        value.node.use_clamp = True
        graph.connect(value, shader.inputs[name])
    bump = graph.node("ShaderNodeBump")
    graph.connect(inputs["Bump Strength"], bump.inputs["Strength"])
    graph.connect(inputs["Bump Size"], bump.inputs["Distance"])
    graph.connect(values["Height"], bump.inputs["Height"])
    graph.connect(bump.outputs["Normal"], shader.inputs["Normal"])
    graph.connect(shader.outputs["BSDF"], outputs["Shader"])
    graph.layout()
    return tree


def main():
    output, name, mode, library, catalog, *paths = sys.argv[sys.argv.index("--") + 1:]
    images = {}
    for label, path in zip(MAPS, paths, strict=True):
        image = bpy.data.images.load(path)
        image.name = f"{name} · {label}"
        image.colorspace_settings.name = "sRGB" if label == "Base Color" else "Non-Color"
        image.pack()
        images[label] = image

    material = bpy.data.materials.new(name)
    if material.node_tree is None:
        material.use_nodes = True
    graph = Graph(material.node_tree)
    graph.tree.nodes.clear()
    group = graph.node("ShaderNodeGroup")
    group.node_tree = material_group(images, mode)
    group.width = 240
    output_node = graph.node("ShaderNodeOutputMaterial")
    graph.connect(group.outputs["Shader"], output_node.inputs["Surface"])
    graph.layout()
    output_node.location.x = 320
    material.asset_mark()
    material.asset_data.description = "Quby PBR material with randomized texture rotation"
    material.asset_data.catalog_id = catalog

    root = Path(library).resolve()
    libraries = bpy.context.preferences.filepaths.asset_libraries
    if not any(item.path and Path(item.path).resolve() == root for item in libraries):
        libraries.new(name="Quby Materials", directory=str(root))
        if "FINISHED" not in bpy.ops.wm.save_userpref():
            raise RuntimeError(f"Could not register Quby Materials in Blender preferences: {root}")
    bpy.data.libraries.write(output, {material}, path_remap="NONE", fake_user=True)


if __name__ == "__main__":
    main()
