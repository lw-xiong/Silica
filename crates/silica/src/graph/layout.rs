//! Force-directed layout for the graph view.
//!
//! Nodes repel each other (Barnes-Hut approximation over a quadtree so a
//! 20k-note vault stays interactive), linked nodes attract along their
//! edges, and everything is pulled gently toward the center. The forces
//! and their default strengths follow `graph.json`: `centerStrength`,
//! `repelStrength`, `linkStrength`, `linkDistance`.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Forces {
    pub center: f32,
    pub repel: f32,
    pub link: f32,
    pub link_distance: f32,
}

impl Default for Forces {
    fn default() -> Forces {
        Forces {
            center: 0.5,
            repel: 10.0,
            link: 1.0,
            link_distance: 250.0,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Body {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    /// Node radius weight: larger nodes repel more and move less.
    pub mass: f32,
    pub pinned: bool,
}

#[derive(Debug, Clone)]
pub struct Layout {
    pub bodies: Vec<Body>,
    /// Pairs of body indexes.
    pub edges: Vec<(u32, u32)>,
    pub forces: Forces,
    /// Simulated annealing temperature; the layout stops when it cools.
    pub alpha: f32,
    pub iterations: u32,
}

const ALPHA_MIN: f32 = 0.003;
const ALPHA_DECAY: f32 = 0.025;
const VELOCITY_DECAY: f32 = 0.6;
const THETA: f32 = 0.9;

impl Layout {
    /// Places `n` nodes on a spiral so the first steps are not degenerate.
    pub fn new(n: usize, edges: Vec<(u32, u32)>, forces: Forces) -> Layout {
        let mut bodies = Vec::with_capacity(n);
        for i in 0..n {
            let angle = i as f32 * 2.399_963; // golden angle
            let radius = 12.0 * (i as f32).sqrt();
            bodies.push(Body {
                x: radius * angle.cos(),
                y: radius * angle.sin(),
                vx: 0.0,
                vy: 0.0,
                mass: 1.0,
                pinned: false,
            });
        }
        Layout {
            bodies,
            edges,
            forces,
            alpha: 1.0,
            iterations: 0,
        }
    }

    pub fn is_settled(&self) -> bool {
        self.alpha < ALPHA_MIN
    }

    /// Reheats after the user drags a node or changes a setting.
    pub fn reheat(&mut self) {
        self.alpha = self.alpha.max(0.3);
    }

    /// One simulation step.
    pub fn step(&mut self) {
        if self.is_settled() || self.bodies.is_empty() {
            return;
        }
        let n = self.bodies.len();
        let alpha = self.alpha;
        let f = self.forces;

        // Repulsion through a quadtree.
        let tree = QuadTree::build(&self.bodies);
        let mut fx = vec![0.0f32; n];
        let mut fy = vec![0.0f32; n];
        for i in 0..n {
            let b = &self.bodies[i];
            tree.apply(b.x, b.y, i, f.repel * 1500.0, &mut fx[i], &mut fy[i]);
        }
        // Attraction along edges.
        for &(a, b) in &self.edges {
            let (a, b) = (a as usize, b as usize);
            if a == b || a >= n || b >= n {
                continue;
            }
            let dx = self.bodies[b].x - self.bodies[a].x;
            let dy = self.bodies[b].y - self.bodies[a].y;
            let dist = (dx * dx + dy * dy).sqrt().max(1.0);
            let pull = (dist - f.link_distance) / dist * f.link * 0.05;
            fx[a] += dx * pull;
            fy[a] += dy * pull;
            fx[b] -= dx * pull;
            fy[b] -= dy * pull;
        }
        // Centering: weak, so a large graph spreads out and the view fits it.
        for (i, b) in self.bodies.iter().enumerate() {
            fx[i] -= b.x * f.center * 0.002;
            fy[i] -= b.y * f.center * 0.002;
        }
        for (i, b) in self.bodies.iter_mut().enumerate() {
            if b.pinned {
                b.vx = 0.0;
                b.vy = 0.0;
                continue;
            }
            let inv = 1.0 / b.mass.max(0.1);
            b.vx = (b.vx + fx[i] * inv * alpha) * VELOCITY_DECAY;
            b.vy = (b.vy + fy[i] * inv * alpha) * VELOCITY_DECAY;
            let speed = (b.vx * b.vx + b.vy * b.vy).sqrt();
            if speed > 40.0 {
                b.vx *= 40.0 / speed;
                b.vy *= 40.0 / speed;
            }
            b.x += b.vx;
            b.y += b.vy;
        }
        self.alpha += (0.0 - self.alpha) * ALPHA_DECAY;
        self.iterations += 1;
    }

    /// Bounding box of all bodies: `(min_x, min_y, max_x, max_y)`.
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let mut out = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for b in &self.bodies {
            out.0 = out.0.min(b.x);
            out.1 = out.1.min(b.y);
            out.2 = out.2.max(b.x);
            out.3 = out.3.max(b.y);
        }
        if self.bodies.is_empty() {
            (0.0, 0.0, 0.0, 0.0)
        } else {
            out
        }
    }
}

struct Node {
    /// Center of mass.
    cx: f32,
    cy: f32,
    mass: f32,
    /// Square cell: top-left corner and side.
    x: f32,
    y: f32,
    side: f32,
    /// Body index when the cell holds exactly one body.
    body: Option<usize>,
    children: Option<[usize; 4]>,
}

struct QuadTree {
    nodes: Vec<Node>,
}

impl QuadTree {
    fn build(bodies: &[Body]) -> QuadTree {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for b in bodies {
            min_x = min_x.min(b.x);
            min_y = min_y.min(b.y);
            max_x = max_x.max(b.x);
            max_y = max_y.max(b.y);
        }
        let side = (max_x - min_x).max(max_y - min_y).max(1.0) + 1.0;
        let mut tree = QuadTree {
            nodes: vec![Node {
                cx: 0.0,
                cy: 0.0,
                mass: 0.0,
                x: min_x - 0.5,
                y: min_y - 0.5,
                side,
                body: None,
                children: None,
            }],
        };
        for (i, b) in bodies.iter().enumerate() {
            tree.insert(0, i, b, bodies, 0);
        }
        tree
    }

    fn insert(&mut self, node: usize, ix: usize, b: &Body, bodies: &[Body], depth: usize) {
        // Accumulate center of mass on the way down.
        {
            let n = &mut self.nodes[node];
            let total = n.mass + b.mass;
            n.cx = (n.cx * n.mass + b.x * b.mass) / total;
            n.cy = (n.cy * n.mass + b.y * b.mass) / total;
            n.mass = total;
        }
        if self.nodes[node].children.is_none() {
            match self.nodes[node].body {
                None => {
                    self.nodes[node].body = Some(ix);
                    return;
                }
                Some(other) if depth > 24 => {
                    // Coincident points: keep the cell as a leaf with merged mass.
                    let _ = other;
                    return;
                }
                Some(other) => {
                    self.subdivide(node);
                    let ob = &bodies[other];
                    let q = self.quadrant(node, ob.x, ob.y);
                    let child = self.nodes[node].children.unwrap()[q];
                    self.nodes[node].body = None;
                    self.insert(child, other, ob, bodies, depth + 1);
                }
            }
        }
        let q = self.quadrant(node, b.x, b.y);
        let child = self.nodes[node].children.unwrap()[q];
        self.insert(child, ix, b, bodies, depth + 1);
    }

    fn subdivide(&mut self, node: usize) {
        let (x, y, half) = {
            let n = &self.nodes[node];
            (n.x, n.y, n.side / 2.0)
        };
        let mut ids = [0usize; 4];
        for (q, id) in ids.iter_mut().enumerate() {
            let (ox, oy) = match q {
                0 => (0.0, 0.0),
                1 => (half, 0.0),
                2 => (0.0, half),
                _ => (half, half),
            };
            *id = self.nodes.len();
            self.nodes.push(Node {
                cx: 0.0,
                cy: 0.0,
                mass: 0.0,
                x: x + ox,
                y: y + oy,
                side: half,
                body: None,
                children: None,
            });
        }
        self.nodes[node].children = Some(ids);
    }

    fn quadrant(&self, node: usize, px: f32, py: f32) -> usize {
        let n = &self.nodes[node];
        let half = n.side / 2.0;
        let right = px >= n.x + half;
        let down = py >= n.y + half;
        match (right, down) {
            (false, false) => 0,
            (true, false) => 1,
            (false, true) => 2,
            (true, true) => 3,
        }
    }

    fn apply(&self, px: f32, py: f32, me: usize, strength: f32, fx: &mut f32, fy: &mut f32) {
        let mut stack = vec![0usize];
        while let Some(id) = stack.pop() {
            let n = &self.nodes[id];
            if n.mass == 0.0 {
                continue;
            }
            let mut dx = px - n.cx;
            let mut dy = py - n.cy;
            if dx == 0.0 && dy == 0.0 {
                // Coincident points: nudge apart deterministically.
                dx = 0.01 * ((me % 7) as f32 + 1.0);
                dy = 0.01 * ((me % 5) as f32 + 1.0);
            }
            let d2 = dx * dx + dy * dy;
            let far = n.side * n.side < THETA * THETA * d2;
            match (&n.children, n.body) {
                (None, Some(b)) if b == me => continue,
                (Some(children), _) if !far => stack.extend(children.iter().copied()),
                _ => {
                    let d2 = d2.max(25.0);
                    let force = strength * n.mass / d2;
                    let d = d2.sqrt();
                    *fx += dx / d * force;
                    *fy += dy / d * force;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist(a: &Body, b: &Body) -> f32 {
        ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
    }

    #[test]
    fn linked_nodes_end_closer_than_unlinked_ones() {
        let forces = Forces {
            link_distance: 80.0,
            ..Forces::default()
        };
        let mut layout = Layout::new(4, vec![(0, 1), (2, 3)], forces);
        for _ in 0..400 {
            layout.step();
        }
        assert!(layout.is_settled());
        let linked = dist(&layout.bodies[0], &layout.bodies[1]);
        let unlinked = dist(&layout.bodies[0], &layout.bodies[2]);
        assert!(linked < unlinked, "linked {linked} unlinked {unlinked}");
        assert!(layout
            .bodies
            .iter()
            .all(|b| b.x.is_finite() && b.y.is_finite()));
    }

    #[test]
    fn coincident_and_pinned_bodies_are_stable() {
        let mut layout = Layout::new(3, vec![(0, 1)], Forces::default());
        layout.bodies[1].x = layout.bodies[0].x;
        layout.bodies[1].y = layout.bodies[0].y;
        layout.bodies[2].pinned = true;
        let (px, py) = (layout.bodies[2].x, layout.bodies[2].y);
        for _ in 0..100 {
            layout.step();
        }
        assert!(layout
            .bodies
            .iter()
            .all(|b| b.x.is_finite() && b.y.is_finite()));
        assert_eq!((layout.bodies[2].x, layout.bodies[2].y), (px, py));
        assert!(dist(&layout.bodies[0], &layout.bodies[1]) > 1.0);
    }

    #[test]
    fn large_graphs_step_quickly() {
        let n = 5000;
        let edges: Vec<(u32, u32)> = (1..n as u32).map(|i| (i, i / 2)).collect();
        let mut layout = Layout::new(n, edges, Forces::default());
        let started = std::time::Instant::now();
        for _ in 0..5 {
            layout.step();
        }
        assert!(started.elapsed().as_millis() < 2000);
        let (x0, y0, x1, y1) = layout.bounds();
        assert!(x1 > x0 && y1 > y0);
    }
}
