//! Flatten a triangle mesh into the plane with the least stretch.

/// A sparse symmetric matrix stored by rows, both triangles kept.
#[derive(Debug, Clone)]
pub struct SymMatrix {
    n: usize,
    rows: Vec<Vec<(usize, f64)>>,
}

impl SymMatrix {
    pub fn new(n: usize) -> Self {
        Self {
            n,
            rows: vec![Vec::new(); n],
        }
    }

    /// Adds `v` to `(i, j)` and, when `i != j`, to `(j, i)`.
    pub fn add(&mut self, i: usize, j: usize, v: f64) {
        self.add_one(i, j, v);
        if i != j {
            self.add_one(j, i, v);
        }
    }

    fn add_one(&mut self, i: usize, j: usize, v: f64) {
        let row = &mut self.rows[i];
        match row.iter_mut().find(|(c, _)| *c == j) {
            Some((_, a)) => *a += v,
            None => row.push((j, v)),
        }
    }

    /// `out = A x`.
    pub fn mul(&self, x: &[f64], out: &mut [f64]) {
        for (i, row) in self.rows.iter().enumerate() {
            out[i] = row.iter().map(|&(j, a)| a * x[j]).sum();
        }
    }

    pub fn diagonal(&self) -> Vec<f64> {
        (0..self.n)
            .map(|i| {
                self.rows[i]
                    .iter()
                    .find(|(c, _)| *c == i)
                    .map_or(0.0, |&(_, a)| a)
            })
            .collect()
    }
}

/// How a conjugate-gradient solve ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CgOutcome {
    pub iterations: usize,
    pub relative_residual: f64,
    pub converged: bool,
}

/// Solves `A x = b` for a symmetric positive-definite `A` by conjugate gradients with a Jacobi
/// (diagonal) preconditioner. `x` is the starting guess and receives the answer. Stops when
/// `‖b − A x‖ / ‖b‖ < tol` or after `max_iter` iterations.
pub fn solve_cg(a: &SymMatrix, b: &[f64], x: &mut [f64], tol: f64, max_iter: usize) -> CgOutcome {
    let n = a.n;
    let dot = |u: &[f64], v: &[f64]| u.iter().zip(v).map(|(p, q)| p * q).sum::<f64>();
    let b_norm = dot(b, b).sqrt();
    if b_norm == 0.0 {
        x.iter_mut().for_each(|v| *v = 0.0);
        return CgOutcome {
            iterations: 0,
            relative_residual: 0.0,
            converged: true,
        };
    }
    let inv_diag: Vec<f64> = a
        .diagonal()
        .iter()
        .map(|&d| if d > 0.0 { 1.0 / d } else { 1.0 })
        .collect();
    let mut ax = vec![0.0; n];
    a.mul(x, &mut ax);
    let mut r: Vec<f64> = b.iter().zip(&ax).map(|(p, q)| p - q).collect();
    let mut z: Vec<f64> = r.iter().zip(&inv_diag).map(|(p, q)| p * q).collect();
    let mut p = z.clone();
    let mut rz = dot(&r, &z);
    let mut ap = vec![0.0; n];
    let mut residual = dot(&r, &r).sqrt() / b_norm;
    let mut it = 0;
    while residual >= tol && it < max_iter {
        a.mul(&p, &mut ap);
        let pap = dot(&p, &ap);
        if pap <= 0.0 || !pap.is_finite() {
            break;
        }
        let alpha = rz / pap;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        it += 1;
        residual = dot(&r, &r).sqrt() / b_norm;
        if residual < tol {
            break;
        }
        for i in 0..n {
            z[i] = r[i] * inv_diag[i];
        }
        let rz_new = dot(&r, &z);
        let beta = rz_new / rz;
        rz = rz_new;
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
    }
    CgOutcome {
        iterations: it,
        relative_residual: residual,
        converged: residual < tol,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3×3 symmetric positive-definite system, solved exactly by Cramer's rule in the test.
    #[test]
    fn cg_solves_a_small_spd_system() {
        let m = [[4.0, 1.0, 0.0], [1.0, 3.0, 1.0], [0.0, 1.0, 2.0]];
        let b = [1.0, 2.0, 3.0];
        let mut a = SymMatrix::new(3);
        for (i, row) in m.iter().enumerate() {
            for (j, &v) in row.iter().enumerate().skip(i) {
                if v != 0.0 {
                    a.add(i, j, v);
                }
            }
        }
        let det3 = |c: [[f64; 3]; 3]| {
            c[0][0] * (c[1][1] * c[2][2] - c[1][2] * c[2][1])
                - c[0][1] * (c[1][0] * c[2][2] - c[1][2] * c[2][0])
                + c[0][2] * (c[1][0] * c[2][1] - c[1][1] * c[2][0])
        };
        let d = det3(m);
        let mut x = vec![0.0; 3];
        let out = solve_cg(&a, &b, &mut x, 1e-14, 100);
        assert!(out.converged, "{out:?}");
        for k in 0..3 {
            let mut c = m;
            for r in 0..3 {
                c[r][k] = b[r];
            }
            let exact = det3(c) / d;
            assert!((x[k] - exact).abs() < 1e-12, "x[{k}] = {} vs {exact}", x[k]);
        }
    }

    /// A path of `n` nodes with unit springs, node 0 held at 0 and a unit load on every other node.
    /// Node `i` carries the load of nodes `i..n`, so `x_i − x_{i−1} = n − i` and
    /// `x_i = Σ_{k=1..i} (n − k) = i(2n − 1 − i)/2` (n = 3: 2, 3).
    #[test]
    fn cg_on_a_path_laplacian_matches_closed_form() {
        let n = 50;
        // Unknowns are nodes 1..n, stored at 0..n-1.
        let mut a = SymMatrix::new(n - 1);
        for e in 0..n - 1 {
            // Edge between node e and node e+1.
            let (p, q) = (e, e + 1);
            if p > 0 {
                a.add(p - 1, p - 1, 1.0);
                a.add(p - 1, q - 1, -1.0);
            }
            a.add(q - 1, q - 1, 1.0);
        }
        let b = vec![1.0; n - 1];
        let mut x = vec![0.0; n - 1];
        let out = solve_cg(&a, &b, &mut x, 1e-14, 10 * n);
        assert!(out.converged, "{out:?}");
        for i in 1..n {
            let exact = (i * (2 * n - 1 - i)) as f64 / 2.0;
            assert!(
                (x[i - 1] - exact).abs() < 1e-9,
                "x_{i} = {} vs {exact}",
                x[i - 1]
            );
        }
    }
}
