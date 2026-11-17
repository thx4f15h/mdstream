//! Current occupied suffixes and current same-base members. Sparse AVL gaps and
//! maximum-owner summaries skip earlier dense claims without numeric rescans.
use super::{
    pages::{Arena, Id},
    runtime::{Cx, Job, Op, boxed},
};
pub(super) type Root = Option<Id>;
#[derive(Clone, Copy)]
struct Node {
    key: usize,
    value: usize,
    left: Root,
    right: Root,
    height: u8,
    count: usize,
    lo: usize,
    hi: usize,
    min: usize,
    max: usize,
}
impl Node {
    fn leaf(key: usize, value: usize) -> Self {
        Self {
            key,
            value,
            left: None,
            right: None,
            height: 1,
            count: 1,
            lo: key,
            hi: key,
            min: value,
            max: value,
        }
    }
}
pub(super) struct Numbers {
    nodes: Arena<Node>,
    cx: Cx,
}
impl Numbers {
    pub fn new(cx: Cx) -> Self {
        Self {
            nodes: Arena::tracked(cx.clone(), 2),
            cx,
        }
    }
    fn height(&self, r: Root) -> u8 {
        r.map_or(0, |id| self.nodes.get(id).height)
    }
    fn fix(&mut self, id: Id) {
        let mut n = *self.nodes.get(id);
        n.height = 1 + self.height(n.left).max(self.height(n.right));
        n.count = 1;
        n.lo = n.key;
        n.hi = n.key;
        n.min = n.value;
        n.max = n.value;
        for child in [n.left, n.right].into_iter().flatten() {
            let c = self.nodes.get(child);
            n.count += c.count;
            n.lo = n.lo.min(c.lo);
            n.hi = n.hi.max(c.hi);
            n.min = n.min.min(c.min);
            n.max = n.max.max(c.max);
        }
        *self.nodes.get_mut(id) = n;
    }
    fn left(&mut self, id: Id) -> Id {
        let x = self.nodes.get(id).right.unwrap();
        let inner = self.nodes.get(x).left;
        self.nodes.get_mut(id).right = inner;
        self.nodes.get_mut(x).left = Some(id);
        self.fix(id);
        self.fix(x);
        x
    }
    fn right(&mut self, id: Id) -> Id {
        let x = self.nodes.get(id).left.unwrap();
        let inner = self.nodes.get(x).right;
        self.nodes.get_mut(id).left = inner;
        self.nodes.get_mut(x).right = Some(id);
        self.fix(id);
        self.fix(x);
        x
    }
    fn balance(&mut self, id: Id) -> Id {
        self.fix(id);
        let n = *self.nodes.get(id);
        if self.height(n.left) > self.height(n.right) + 1 {
            let l = n.left.unwrap();
            let ln = *self.nodes.get(l);
            if self.height(ln.left) < self.height(ln.right) {
                let rotated = self.left(l);
                self.nodes.get_mut(id).left = Some(rotated);
            }
            return self.right(id);
        }
        if self.height(n.right) > self.height(n.left) + 1 {
            let r = n.right.unwrap();
            let rn = *self.nodes.get(r);
            if self.height(rn.right) < self.height(rn.left) {
                let rotated = self.right(r);
                self.nodes.get_mut(id).right = Some(rotated);
            }
            return self.left(id);
        }
        id
    }
    pub fn put(&mut self, root: Root, key: usize, value: usize) -> Job<'_, Root> {
        boxed(&self.cx.clone(), async move {
            self.cx.charge(Op::Numeric).await;
            let Some(id) = root else {
                return Some(self.nodes.insert(Node::leaf(key, value)));
            };
            let n = *self.nodes.get(id);
            if key < n.key {
                let child = self.put(n.left, key, value).await;
                self.nodes.get_mut(id).left = child;
            } else if key > n.key {
                let child = self.put(n.right, key, value).await;
                self.nodes.get_mut(id).right = child;
            } else {
                self.nodes.get_mut(id).value = value;
            }
            Some(self.balance(id))
        })
    }
    pub fn remove(&mut self, root: Root, key: usize) -> Job<'_, Root> {
        boxed(&self.cx.clone(), async move {
            self.cx.charge(Op::Numeric).await;
            let id = root?;
            let n = *self.nodes.get(id);
            if key < n.key {
                let child = self.remove(n.left, key).await;
                self.nodes.get_mut(id).left = child;
            } else if key > n.key {
                let child = self.remove(n.right, key).await;
                self.nodes.get_mut(id).right = child;
            } else {
                if n.left.is_none() || n.right.is_none() {
                    self.nodes.remove(id);
                    return n.left.or(n.right);
                }
                let mut successor = n.right.unwrap();
                loop {
                    self.cx.charge(Op::Numeric).await;
                    if let Some(left) = self.nodes.get(successor).left {
                        successor = left;
                    } else {
                        break;
                    }
                }
                let s = *self.nodes.get(successor);
                let child = self.remove(n.right, s.key).await;
                let item = self.nodes.get_mut(id);
                item.key = s.key;
                item.value = s.value;
                item.right = child;
            }
            Some(self.balance(id))
        })
    }
    fn available(&self, root: Root, owner: usize, at: usize) -> Job<'_, (Option<usize>, usize)> {
        boxed(&self.cx.clone(), async move {
            self.cx.charge(Op::Numeric).await;
            let Some(id) = root else {
                return (None, at);
            };
            let n = *self.nodes.get(id);
            if n.hi < at {
                return (None, at);
            }
            if n.lo > at {
                return (Some(at), at);
            }
            if n.lo >= at && n.hi - n.lo == n.count - 1 && n.max < owner {
                return (None, n.hi.checked_add(1).unwrap());
            }
            let (found, mut at) = self.available(n.left, owner, at).await;
            if found.is_some() {
                return (found, at);
            }
            if n.key >= at {
                if n.key > at || n.value >= owner {
                    return (Some(at), at);
                }
                at += 1;
            }
            self.available(n.right, owner, at).await
        })
    }
    pub async fn first_available(&self, root: Root, owner: usize) -> usize {
        let (found, end) = self.available(root, owner, 0).await;
        found.unwrap_or(end)
    }
    pub fn min_after(&self, root: Root, key: usize) -> Job<'_, Option<usize>> {
        boxed(&self.cx.clone(), async move {
            self.cx.charge(Op::Numeric).await;
            let id = root?;
            let n = *self.nodes.get(id);
            if n.hi <= key {
                return None;
            }
            if n.lo > key {
                return Some(n.min);
            }
            let mut result = (n.key > key).then_some(n.value);
            for child in [n.left, n.right] {
                if let Some(next) = self.min_after(child, key).await {
                    result = Some(result.map_or(next, |old| old.min(next)));
                }
            }
            result
        })
    }
}
