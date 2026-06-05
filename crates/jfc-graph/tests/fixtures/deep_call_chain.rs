// Expected nodes: 10 Functions (a through j)
// Expected edges: a->b, b->c, c->d, d->e, e->f, f->g, g->h, h->i, i->j (9 Calls edges)
// Used to test depth limiting: depth=2 from a should yield {a, b, c}

pub fn a() -> i32 {
    b()
}
fn b() -> i32 {
    c()
}
fn c() -> i32 {
    d()
}
fn d() -> i32 {
    e()
}
fn e() -> i32 {
    f()
}
fn f() -> i32 {
    g()
}
fn g() -> i32 {
    h()
}
fn h() -> i32 {
    i()
}
fn i() -> i32 {
    j()
}
fn j() -> i32 {
    42
}
