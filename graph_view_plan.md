# Building Prompt: Futuristic Knowledge Graph Visualization Engine

Your task is to build a **high-performance interactive knowledge graph visualization** that resembles a living neural network rather than a traditional graph. The goal is to produce a visualization similar to AI brain maps commonly seen in research papers, NVIDIA demos, and futuristic interfaces.

The graph must **never collapse into a single dense ball**. Instead, it should naturally organize itself into communities with visible empty space between them while maintaining aesthetically pleasing interconnections.

---

# Overall Design Philosophy

The graph should feel like:

* an artificial brain
* a galaxy of knowledge
* interconnected neurons
* a living memory system

Every movement should appear organic.

Nodes should gently float into equilibrium instead of instantly jumping.

Edges should resemble neural connections.

Clusters should breathe and slightly oscillate while remaining stable.

There should always be visible negative space between communities.

---

# Graph Layout Algorithm

Use a **multi-stage layout pipeline** instead of relying on a single force-directed simulation.

The recommended pipeline is:

## Stage 1 — Community Detection

First detect communities before any positioning.

Possible algorithms:

* Leiden
* Louvain
* Label Propagation
* Infomap

Each detected community becomes a "super node".

Example:

```
Memory
 ├── People
 ├── Projects
 ├── Places

Programming
 ├── Rust
 ├── C#
 ├── Python

Hardware
 ├── Raspberry Pi
 ├── ESP32
 ├── Cameras
```

These should become three large graph regions.

---

## Stage 2 — Cluster Placement

Do NOT randomly position every node.

Instead:

Treat every community as a planet.

Distribute planets using:

* Circle Packing
* Voronoi Partitioning
* Force Atlas 2 (cluster level)
* Fruchterman-Reingold (cluster only)

The objective is:

* clusters never overlap
* balanced empty space
* aesthetically pleasing distribution
* long bridges between communities

Example:

```
          Programming

Hardware          AI

     Memory

         Robotics

Knowledge

```

---

## Stage 3 — Internal Cluster Layout

Once clusters are positioned, layout nodes inside each cluster separately.

Recommended algorithms:

* ForceAtlas2
* Barnes-Hut
* Fruchterman-Reingold

Use a much stronger repulsion inside clusters.

Each cluster should resemble a small neural cloud.

Never create perfect circles.

Irregular shapes look more natural.

---

## Stage 4 — Bridge Optimization

Most edges should stay inside their own cluster.

Only a few important nodes should connect clusters.

Those become bridge nodes.

Example

```
Rust
    \
     AI Assistant
        \
      Raspberry Pi

```

Avoid spaghetti.

Reduce edge crossings.

Prefer long elegant connections.

---

# Positioning Rules

Every node has:

```
position
velocity
mass
cluster
importance
degree
```

Important nodes should migrate toward the center of their own cluster.

Peripheral nodes stay near the outside.

High-degree nodes become local hubs.

Low-degree nodes orbit around them.

This naturally creates a brain-like structure.

---

# Repulsion Rules

Repulsion must depend on hierarchy.

Example

```
same node:
strong

same cluster:
medium

different clusters:
very strong

```

This keeps clusters separated.

---

# Attraction Rules

Edge attraction depends on edge weight.

Strong relationships:

```
==================
```

Weak relationships:

```
---------
```

Only neighboring nodes should pull each other.

---

# Cluster Shape

Clusters should NOT become circles.

Instead aim for

* clouds
* blobs
* neuron groups
* galaxies

Each cluster should have an organic outline.

---

# Edge Rendering

Edges should be semi-transparent.

Opacity:

```
weight = high
opacity = 0.8

weight = low
opacity = 0.15
```

Use additive blending.

Edges should create glowing webs.

---

# Node Rendering

Different node sizes.

Example

```
Major concept
●●●●●●

Medium
●●●

Small
●

```

Size should depend on

* PageRank
* Degree
* Centrality
* Importance score

---

# Node Colors

Color entire communities.

Example

```
Programming
Blue

AI
Purple

Hardware
Orange

Memory
Green

People
Red

```

Nodes inside a cluster share similar hues.

---

# Physics Parameters

Recommended starting values

```
Repulsion:
2000–6000

Spring length:
60–180

Spring strength:
0.03

Velocity decay:
0.85

Damping:
0.92

Barnes-Hut theta:
0.5

Collision radius:
nodeRadius + padding

```

---

# Animation

Movement should never be chaotic.

Nodes should:

* smoothly interpolate
* slightly float
* gently oscillate
* settle naturally

Avoid jitter.

---

# Camera Behavior

Support

* zoom
* pan
* inertia
* smooth transitions

Double click:

focus cluster

Double click node:

center node

Mouse wheel:

animated zoom

---

# Visual Effects

Background:

dark space

Tiny stars

Subtle particles

Soft bloom

Glow around nodes

Edges fade into darkness

The graph should resemble a living neural network suspended in space.

---

# Labels

Do not display every label.

Rules

```
Zoomed out

Only largest hubs

Zoom level 1

Major nodes

Zoom level 2

Most labels

Zoom level 3

All labels

```

This prevents clutter.

---

# Interaction

Hover:

highlight neighborhood

Click:

pin node

Double click:

expand neighborhood

Shift click:

multi-select

Search:

fly camera to node

---

# Performance

Target:

100,000+ nodes

Millions of edges

Use:

* Barnes-Hut optimization
* QuadTree
* Spatial hashing
* WebGL rendering
* GPU instancing
* Edge batching
* Level-of-detail (LOD)
* Progressive loading

Never redraw everything each frame.

---

# Desired Visual Reference

The final visualization should resemble:

* NVIDIA WholeGraph demos
* ForceAtlas2 network visualizations
* Gephi organic layouts
* Cosmograph
* Graphistry
* Neo4j Bloom
* AI "brain map" interfaces seen in sci-fi UI concepts

The result should look less like a database diagram and more like a **living digital brain**, where clusters form naturally separated neural regions connected by elegant glowing bridges, maintaining both readability and visual impact even as the graph grows.
