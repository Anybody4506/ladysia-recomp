//! Routines arithmétiques de la bibliothèque C du compilateur (libgcc, Thumb).
//! Division par zéro : la routine d'erreur du jeu (0x08035D08) ne fait rien, résultat 0.

use super::Env;

/// Division signée de `__divsi3` (0 si `b` = 0).
pub fn div(a: i32, b: i32) -> i32 {
    if b == 0 {
        return 0;
    }
    let q = a.unsigned_abs() / b.unsigned_abs();
    (if (a ^ b) < 0 { q.wrapping_neg() } else { q }) as i32
}

/// `__divsi3(a, b)` (0x08035C74) : division signée, arrondie vers zéro.
pub fn divsi3(env: &mut Env) {
    env.cpu.r[0] = div(env.cpu.r[0] as i32, env.cpu.r[1] as i32) as u32;
}

/// `__modsi3(a, b)` (0x08035D40) : reste signé (du signe de `a`).
pub fn modsi3(env: &mut Env) {
    let (a, b) = (env.cpu.r[0] as i32, env.cpu.r[1] as i32);
    env.cpu.r[0] = if b == 0 {
        0
    } else {
        let r = a.unsigned_abs() % b.unsigned_abs();
        if a < 0 { r.wrapping_neg() } else { r }
    };
}

/// `__udivsi3(a, b)` (0x08036270) : division non signée.
pub fn udivsi3(env: &mut Env) {
    let (a, b) = (env.cpu.r[0], env.cpu.r[1]);
    env.cpu.r[0] = if b == 0 { 0 } else { a / b };
}

/// `__umodsi3(a, b)` (0x080362E8) : reste non signé.
pub fn umodsi3(env: &mut Env) {
    let (a, b) = (env.cpu.r[0], env.cpu.r[1]);
    env.cpu.r[0] = if b == 0 { 0 } else { a % b };
}

/// `__muldi3(a, b)` (0x08035E10) : produit 64 bits (r1:r0 × r3:r2), résultat dans r1:r0.
pub fn muldi3(env: &mut Env) {
    let a = (env.cpu.r[1] as u64) << 32 | env.cpu.r[0] as u64;
    let b = (env.cpu.r[3] as u64) << 32 | env.cpu.r[2] as u64;
    let p = a.wrapping_mul(b);
    env.cpu.r[0] = p as u32;
    env.cpu.r[1] = (p >> 32) as u32;
}
