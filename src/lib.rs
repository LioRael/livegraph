//! Typed resource graphs with incremental computation and atomic snapshot publication.
//!
//! Build callbacks must be pure, synchronous and non-reentrant. Values must be
//! logically immutable: interior mutation of payloads bypasses snapshot isolation.
//! Read related values through one [`Snapshot`], not separate `snapshot()` calls.
//!
//! ```
//! use livegraph::{GraphBuilder, Result};
//! # fn main() -> Result<()> {
//! let mut b = GraphBuilder::new();
//! let count = b.resource("count", 2_u32);
//! let doubled = b.derived("doubled", [count.dep()], move |cx| {
//!     Ok(*cx.get(count)? * 2)
//! })?;
//! let graph = b.build()?;
//! let old = graph.snapshot();
//! let mut changes = graph.changes();
//! changes.set(count, 3)?;
//! let new = graph.commit(changes)?;
//! assert_eq!(*old.get(doubled)?, 4);
//! assert_eq!(*new.get(doubled)?, 6);
//! # Ok(()) }
//! ```

use arc_swap::ArcSwap;
use std::{
    any::Any,
    cell::RefCell,
    collections::{BTreeMap, VecDeque},
    fmt,
    marker::PhantomData,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

/// A graph definition, computation, validation or access error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A handle or change set belongs to a different graph.
    ForeignGraph,
    /// An invalid node index was used.
    UnknownNode(usize),
    /// A reserved node was not defined.
    Undefined(String),
    /// A node was defined more than once.
    AlreadyDefined(String),
    /// Graph contains a cycle; listed nodes are blocked by cycles (including descendants).
    Cycle(Vec<String>),
    /// A callback attempted to read a dependency it did not declare.
    UndeclaredDependency(usize),
    /// Only resource nodes can be assigned.
    NotResource(String),
    /// A callback rejected a value or candidate snapshot.
    Message(String),
    /// Computation failed at the named node.
    Build {
        /// Name of the failed derived node.
        node: String,
        /// Callback error description.
        reason: String,
    },
    /// A named snapshot validator failed.
    Validation {
        /// Name of the failed validator.
        name: String,
        /// Validator error description.
        reason: String,
    },
    /// A callback panicked; no candidate was published.
    CallbackPanicked(String),
    /// A writer previously unwound outside a callback.
    WriterPoisoned,
    /// The same thread attempted to commit recursively to this graph.
    ReentrantCommit,
    /// A checked batch was based on a version that is no longer current.
    Conflict {
        /// Version used to prepare the batch.
        expected: u64,
        /// Current committed version.
        actual: u64,
    },
    /// The snapshot version cannot be incremented.
    VersionExhausted,
    /// Internal erased value did not match the typed handle.
    TypeMismatch(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ForeignGraph => write!(f, "handle or changes belong to another graph"),
            Self::UnknownNode(i) => write!(f, "unknown node {i}"),
            Self::Undefined(n) => write!(f, "undefined node: {n}"),
            Self::AlreadyDefined(n) => write!(f, "node already defined: {n}"),
            Self::Cycle(ns) => write!(f, "cycle blocks nodes: {}", ns.join(", ")),
            Self::UndeclaredDependency(i) => write!(f, "undeclared dependency: {i}"),
            Self::NotResource(n) => write!(f, "not a resource: {n}"),
            Self::Message(m) => f.write_str(m),
            Self::Build { node, reason } => write!(f, "build {node}: {reason}"),
            Self::Validation { name, reason } => write!(f, "validation {name}: {reason}"),
            Self::CallbackPanicked(n) => write!(f, "callback panicked: {n}"),
            Self::ReentrantCommit => write!(f, "recursive commit to the same graph"),
            Self::Conflict { expected, actual } => write!(
                f,
                "snapshot conflict: expected v{expected}, current v{actual}"
            ),
            Self::WriterPoisoned => write!(f, "writer lock poisoned"),
            Self::VersionExhausted => write!(f, "snapshot version exhausted"),
            Self::TypeMismatch(i) => write!(f, "type mismatch at node {i}"),
        }
    }
}
impl std::error::Error for Error {}
impl From<String> for Error {
    fn from(value: String) -> Self {
        Self::Message(value)
    }
}
impl From<&str> for Error {
    fn from(value: &str) -> Self {
        Self::Message(value.into())
    }
}

/// Result returned by graph operations and user callbacks.
pub type Result<T> = std::result::Result<T, Error>;
type Value = Arc<dyn Any + Send + Sync>;
type Compute = Box<dyn Fn(&Context<'_>) -> Result<Value> + Send + Sync>;
type Validate = Box<dyn Fn(&Snapshot) -> Result<()> + Send + Sync>;
static NEXT_GRAPH: AtomicU64 = AtomicU64::new(1);

/// A typed, graph-scoped node handle. Copying it never copies a value.
///
/// Resource replacements must have the handle's value type:
/// ```compile_fail
/// use livegraph::GraphBuilder;
/// let mut builder = GraphBuilder::new();
/// let count = builder.resource("count", 1_u32);
/// let graph = builder.build().unwrap();
/// graph.changes().set(count, String::from("wrong type")).unwrap();
/// ```
pub struct Node<T> {
    graph: u64,
    index: usize,
    // Invariance prevents a handle's type from being changed through subtyping.
    marker: PhantomData<fn(T) -> T>,
}
impl<T> Copy for Node<T> {}
impl<T> Clone for Node<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> fmt::Debug for Node<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Node")
            .field("graph", &self.graph)
            .field("index", &self.index)
            .finish()
    }
}
impl<T> Node<T> {
    /// Erase the value type for an explicit dependency list.
    pub fn dep(self) -> Dependency {
        Dependency {
            graph: self.graph,
            index: self.index,
        }
    }
}

/// An opaque graph-scoped dependency, obtained from [`Node::dep`].
#[derive(Debug, Clone, Copy)]
pub struct Dependency {
    graph: u64,
    index: usize,
}

struct Definition {
    name: String,
    deps: Vec<usize>,
    kind: Kind,
}
enum Kind {
    Undefined,
    Resource(Option<Value>),
    Derived(Compute),
}

const CHUNK_SIZE: usize = 16;
type Chunk = [Option<Value>; CHUNK_SIZE];

#[derive(Clone)]
struct Values {
    len: usize,
    chunks: Vec<Arc<Chunk>>,
}
impl Values {
    fn empty(len: usize) -> Self {
        Self {
            len,
            chunks: (0..len.div_ceil(CHUNK_SIZE))
                .map(|_| Arc::new(std::array::from_fn(|_| None)))
                .collect(),
        }
    }
    fn fork(&self, affected: impl Iterator<Item = usize>) -> Self {
        let mut replaced = vec![0_usize; self.chunks.len()];
        for index in affected {
            replaced[index / CHUNK_SIZE] += 1;
        }
        let chunks = self
            .chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| {
                let count = CHUNK_SIZE.min(self.len - i * CHUNK_SIZE);
                if replaced[i] == count {
                    // Every real slot will be replaced before publication. A blank block
                    // avoids copying values that would immediately be overwritten.
                    Arc::new(std::array::from_fn(|_| None))
                } else {
                    chunk.clone()
                }
            })
            .collect();
        Self {
            len: self.len,
            chunks,
        }
    }
    fn len(&self) -> usize {
        self.len
    }
    fn get(&self, index: usize) -> Option<&Option<Value>> {
        if index >= self.len {
            return None;
        }
        Some(&self.chunks[index / CHUNK_SIZE][index % CHUNK_SIZE])
    }
    // ALL is used only for Values::empty candidates; no block reference has
    // escaped before publication. Partial candidates must use copy-on-write.
    fn write<const ALL: bool>(&mut self, index: usize, value: Value) {
        if ALL {
            Arc::get_mut(&mut self.chunks[index / CHUNK_SIZE])
                .expect("fully affected candidates have exclusively owned blocks")
                [index % CHUNK_SIZE] = Some(value);
        } else {
            self.set(index, value);
        }
    }
    fn set(&mut self, index: usize, value: Value) {
        debug_assert!(index < self.len);
        Arc::make_mut(&mut self.chunks[index / CHUNK_SIZE])[index % CHUNK_SIZE] = Some(value);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UpdateStrategy {
    Sparse,
    Dense,
}

// Writer-local scratch state contains no payloads. Epochs let a failed build leave
// marks behind without clearing V entries before the next sparse update.
struct Scheduler {
    seen: Vec<u64>,
    epoch: u64,
    pending: Vec<usize>,
}
impl Scheduler {
    fn new(nodes: usize) -> Self {
        Self {
            seen: vec![0; nodes],
            epoch: 0,
            pending: Vec::new(),
        }
    }
    fn affected(
        &mut self,
        roots: impl ExactSizeIterator<Item = usize>,
        outgoing: &[Vec<usize>],
        rank: &[usize],
        order: &[usize],
    ) -> UpdateStrategy {
        self.epoch = match self.epoch.checked_add(1) {
            Some(next) => next,
            None => {
                self.seen.fill(0);
                1
            }
        };
        self.pending.clear();
        let dense_threshold = order.len().div_ceil(4).max(1);
        let dense_roots = roots.len() >= dense_threshold;
        for root in roots {
            if self.seen[root] != self.epoch {
                self.seen[root] = self.epoch;
                if !dense_roots {
                    self.pending.push(rank[root]);
                }
            }
        }
        let mut cursor = 0;
        while !dense_roots && cursor < self.pending.len() {
            let node = order[self.pending[cursor]];
            for &next in &outgoing[node] {
                if self.seen[next] != self.epoch {
                    self.seen[next] = self.epoch;
                    self.pending.push(rank[next]);
                }
            }
            cursor += 1;
        }
        if dense_roots {
            // Choose once from the staged roots, a lower bound on affected nodes.
            // Do not restart a partial traversal: a single broadcast root would
            // visit its high-fanout edges twice. Unaffected nodes are never evaluated.
            self.pending.clear();
            for (position, &node) in order.iter().enumerate() {
                if self.seen[node] == self.epoch {
                    self.pending.push(position);
                    for &next in &outgoing[node] {
                        self.seen[next] = self.epoch;
                    }
                }
            }
            UpdateStrategy::Dense
        } else {
            self.pending.sort_unstable();
            UpdateStrategy::Sparse
        }
    }
}

/// A frozen, versioned set of node values. Unchanged payloads are shared across versions.
///
/// Payloads must not use interior mutation to change graph-visible state.
pub struct Snapshot {
    graph: u64,
    version: u64,
    values: Values,
}
impl fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Snapshot")
            .field("graph", &self.graph)
            .field("version", &self.version)
            .field("nodes", &self.values.len())
            .finish()
    }
}
impl Snapshot {
    /// Monotonic successful-commit version. Initial state is version zero.
    pub fn version(&self) -> u64 {
        self.version
    }
    /// Read a typed value from this exact version.
    pub fn get<T: Send + Sync + 'static>(&self, node: Node<T>) -> Result<&T> {
        if node.graph != self.graph {
            return Err(Error::ForeignGraph);
        }
        self.values
            .get(node.index)
            .ok_or(Error::UnknownNode(node.index))?
            .as_ref()
            .and_then(|v| v.downcast_ref())
            .ok_or(Error::TypeMismatch(node.index))
    }
}

/// Restricted view of the candidate's declared direct dependencies.
pub struct Context<'a> {
    snapshot: &'a Snapshot,
    deps: &'a [usize],
}
impl Context<'_> {
    /// Read a declared dependency after it has been evaluated for this candidate.
    pub fn get<T: Send + Sync + 'static>(&self, node: Node<T>) -> Result<&T> {
        if node.graph != self.snapshot.graph {
            return Err(Error::ForeignGraph);
        }
        if self.deps.binary_search(&node.index).is_err() {
            return Err(Error::UndeclaredDependency(node.index));
        }
        self.snapshot.get(node)
    }
}

/// Builds a fixed graph. Forward references are supported with [`Self::reserve`].
pub struct GraphBuilder {
    id: u64,
    nodes: Vec<Definition>,
    validators: Vec<(String, Validate)>,
}
impl Default for GraphBuilder {
    fn default() -> Self {
        Self::new()
    }
}
impl GraphBuilder {
    /// Create an independent graph definition.
    pub fn new() -> Self {
        let mut id = NEXT_GRAPH.load(Ordering::Relaxed);
        loop {
            let next = id.checked_add(1).expect("graph identity space exhausted");
            match NEXT_GRAPH.compare_exchange_weak(id, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => break,
                Err(current) => id = current,
            }
        }
        Self {
            id,
            nodes: Vec::new(),
            validators: Vec::new(),
        }
    }
    /// Reserve a typed node for forward references. It must be defined before build.
    pub fn reserve<T: Send + Sync + 'static>(&mut self, name: impl Into<String>) -> Node<T> {
        let index = self.nodes.len();
        self.nodes.push(Definition {
            name: name.into(),
            deps: vec![],
            kind: Kind::Undefined,
        });
        Node {
            graph: self.id,
            index,
            marker: PhantomData,
        }
    }
    fn check(&self, dep: Dependency) -> Result<usize> {
        if dep.graph != self.id {
            return Err(Error::ForeignGraph);
        }
        if dep.index >= self.nodes.len() {
            return Err(Error::UnknownNode(dep.index));
        }
        Ok(dep.index)
    }
    fn undefined<T>(&self, node: Node<T>) -> Result<usize> {
        let i = self.check(node.dep())?;
        if !matches!(self.nodes[i].kind, Kind::Undefined) {
            return Err(Error::AlreadyDefined(self.nodes[i].name.clone()));
        }
        Ok(i)
    }
    /// Add a resource with its initial value.
    pub fn resource<T: Send + Sync + 'static>(
        &mut self,
        name: impl Into<String>,
        value: T,
    ) -> Node<T> {
        let node = self.reserve(name);
        self.nodes[node.index].kind = Kind::Resource(Some(Arc::new(value)));
        node
    }
    /// Define an already reserved node as a resource.
    pub fn define_resource<T: Send + Sync + 'static>(
        &mut self,
        node: Node<T>,
        value: T,
    ) -> Result<()> {
        let i = self.undefined(node)?;
        self.nodes[i].kind = Kind::Resource(Some(Arc::new(value)));
        Ok(())
    }
    fn dependencies(&self, deps: impl IntoIterator<Item = Dependency>) -> Result<Vec<usize>> {
        let mut deps = deps
            .into_iter()
            .map(|d| self.check(d))
            .collect::<Result<Vec<_>>>()?;
        deps.sort_unstable();
        deps.dedup();
        Ok(deps)
    }
    /// Add a derived node. The callback can only read the listed dependencies.
    pub fn derived<T, F>(
        &mut self,
        name: impl Into<String>,
        deps: impl IntoIterator<Item = Dependency>,
        compute: F,
    ) -> Result<Node<T>>
    where
        T: Send + Sync + 'static,
        F: Fn(&Context<'_>) -> Result<T> + Send + Sync + 'static,
    {
        let deps = self.dependencies(deps)?;
        let node = self.reserve(name);
        self.install(node, deps, compute);
        Ok(node)
    }
    /// Define a reserved node as derived, permitting forward references and cycle checks.
    pub fn define_derived<T, F>(
        &mut self,
        node: Node<T>,
        deps: impl IntoIterator<Item = Dependency>,
        compute: F,
    ) -> Result<()>
    where
        T: Send + Sync + 'static,
        F: Fn(&Context<'_>) -> Result<T> + Send + Sync + 'static,
    {
        self.undefined(node)?;
        let deps = self.dependencies(deps)?;
        self.install(node, deps, compute);
        Ok(())
    }
    fn install<T, F>(&mut self, node: Node<T>, deps: Vec<usize>, compute: F)
    where
        T: Send + Sync + 'static,
        F: Fn(&Context<'_>) -> Result<T> + Send + Sync + 'static,
    {
        self.nodes[node.index].deps = deps;
        self.nodes[node.index].kind = Kind::Derived(Box::new(move |cx| {
            compute(cx).map(|v| Arc::new(v) as Value)
        }));
    }
    /// Add a whole-snapshot invariant, checked initially and before every nonempty commit.
    pub fn validate<F>(&mut self, name: impl Into<String>, validate: F)
    where
        F: Fn(&Snapshot) -> Result<()> + Send + Sync + 'static,
    {
        self.validators.push((name.into(), Box::new(validate)));
    }
    /// Check all definitions and cycles, evaluate all nodes, then validate the initial snapshot.
    pub fn build(mut self) -> Result<LiveGraph> {
        for n in &self.nodes {
            if matches!(n.kind, Kind::Undefined) {
                return Err(Error::Undefined(n.name.clone()));
            }
        }
        let mut outgoing = vec![vec![]; self.nodes.len()];
        let mut indegree: Vec<_> = self.nodes.iter().map(|n| n.deps.len()).collect();
        for (i, n) in self.nodes.iter().enumerate() {
            for &d in &n.deps {
                outgoing[d].push(i);
            }
        }
        let mut ready: VecDeque<_> = indegree
            .iter()
            .enumerate()
            .filter_map(|(i, &d)| (d == 0).then_some(i))
            .collect();
        let mut order = Vec::new();
        while let Some(i) = ready.pop_front() {
            order.push(i);
            for &next in &outgoing[i] {
                indegree[next] -= 1;
                if indegree[next] == 0 {
                    ready.push_back(next);
                }
            }
        }
        if order.len() != self.nodes.len() {
            return Err(Error::Cycle(
                indegree
                    .iter()
                    .enumerate()
                    .filter(|(_, d)| **d > 0)
                    .map(|(i, _)| self.nodes[i].name.clone())
                    .collect(),
            ));
        }
        let mut initial = Snapshot {
            graph: self.id,
            version: 0,
            values: Values::empty(self.nodes.len()),
        };
        for &i in &order {
            let value = match &self.nodes[i].kind {
                Kind::Resource(v) => v.as_ref().expect("resource initialized").clone(),
                Kind::Derived(f) => evaluate(&self.nodes[i], f, &initial)?,
                Kind::Undefined => unreachable!(),
            };
            initial.values.set(i, value);
        }
        validate_all(&self.validators, &initial)?;
        // The definition must not retain resource payloads after publication.
        for node in &mut self.nodes {
            if let Kind::Resource(value) = &mut node.kind {
                value.take();
            }
        }
        let mut rank = vec![0; self.nodes.len()];
        for (position, &node) in order.iter().enumerate() {
            rank[node] = position;
        }
        let scheduler = Scheduler::new(self.nodes.len());
        Ok(LiveGraph {
            id: self.id,
            nodes: self.nodes.into(),
            order,
            rank,
            outgoing,
            validators: self.validators,
            current: ArcSwap::from_pointee(initial),
            writer: Mutex::new(scheduler),
        })
    }
}

fn evaluate(node: &Definition, f: &Compute, snapshot: &Snapshot) -> Result<Value> {
    catch_unwind(AssertUnwindSafe(|| {
        f(&Context {
            snapshot,
            deps: &node.deps,
        })
    }))
    .map_err(|_| Error::CallbackPanicked(node.name.clone()))?
    .map_err(|e| Error::Build {
        node: node.name.clone(),
        reason: e.to_string(),
    })
}
fn validate_all(validators: &[(String, Validate)], snapshot: &Snapshot) -> Result<()> {
    for (name, f) in validators {
        catch_unwind(AssertUnwindSafe(|| f(snapshot)))
            .map_err(|_| Error::CallbackPanicked(name.clone()))?
            .map_err(|e| Error::Validation {
                name: name.clone(),
                reason: e.to_string(),
            })?;
    }
    Ok(())
}

/// A batch of resource replacements. Repeated assignments use the last value.
///
/// [`LiveGraph::changes`] applies replacements to the latest version under the writer
/// lock. [`LiveGraph::changes_from`] additionally checks the source snapshot version.
pub struct Changes {
    graph: u64,
    nodes: Arc<[Definition]>,
    expected_version: Option<u64>,
    error: Option<Error>,
    values: BTreeMap<usize, Value>,
}
impl Changes {
    /// Stage a typed resource replacement; derived and foreign nodes fail immediately.
    ///
    /// Any staging error invalidates the entire batch. Later sets and commit return
    /// that error, so ignoring one failed set cannot publish a partial batch.
    pub fn set<T: Send + Sync + 'static>(&mut self, node: Node<T>, value: T) -> Result<&mut Self> {
        let check = || {
            if let Some(error) = &self.error {
                return Err(error.clone());
            }
            if node.graph != self.graph {
                return Err(Error::ForeignGraph);
            }
            let definition = self
                .nodes
                .get(node.index)
                .ok_or(Error::UnknownNode(node.index))?;
            if !matches!(definition.kind, Kind::Resource(_)) {
                return Err(Error::NotResource(definition.name.clone()));
            }
            Ok(())
        };
        if let Err(error) = check() {
            self.error = Some(error.clone());
            return Err(error);
        }
        self.values.insert(node.index, Arc::new(value));
        Ok(self)
    }
}

/// A fixed dependency graph with serialized writers and concurrent snapshot readers.
pub struct LiveGraph {
    id: u64,
    nodes: Arc<[Definition]>,
    order: Vec<usize>,
    rank: Vec<usize>,
    outgoing: Vec<Vec<usize>>,
    validators: Vec<(String, Validate)>,
    current: ArcSwap<Snapshot>,
    writer: Mutex<Scheduler>,
}
impl fmt::Debug for LiveGraph {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveGraph")
            .field("graph", &self.id)
            .field("nodes", &self.nodes.len())
            .field("version", &self.snapshot().version())
            .finish()
    }
}

thread_local! {
    static ACTIVE_COMMITS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
}
struct ActiveCommit(u64);
impl ActiveCommit {
    fn enter(graph: u64) -> Result<Self> {
        ACTIVE_COMMITS.with(|active| {
            let mut active = active.borrow_mut();
            if active.contains(&graph) {
                return Err(Error::ReentrantCommit);
            }
            active.push(graph);
            Ok(Self(graph))
        })
    }
}
impl Drop for ActiveCommit {
    fn drop(&mut self) {
        ACTIVE_COMMITS.with(|active| {
            let popped = active.borrow_mut().pop();
            debug_assert_eq!(popped, Some(self.0));
        });
    }
}
impl LiveGraph {
    /// Acquire one consistent version, without taking the writer mutex.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.current.load_full()
    }
    /// Borrow one committed version for a short synchronous callback.
    ///
    /// The callback runs exactly once without acquiring the writer mutex. All reads
    /// through its argument see the same version, even while writers publish newer
    /// snapshots. Prefer [`Self::snapshot`] for long-lived or cross-thread ownership.
    /// Nested reads may exhaust arc-swap's fast borrow slots and use its owned
    /// fallback; this affects cost, not consistency. A callback panic propagates and
    /// releases the guard during unwinding; this is not a transaction for side effects.
    ///
    /// ```
    /// use livegraph::{GraphBuilder, Result};
    /// # fn main() -> Result<()> {
    /// let mut builder = GraphBuilder::new();
    /// let count = builder.resource("count", 7_u64);
    /// let graph = builder.build()?;
    /// let value = graph.with_snapshot(|snapshot| snapshot.get(count).copied())?;
    /// assert_eq!(value, 7);
    /// # Ok(()) }
    /// ```
    /// References borrowed from the snapshot cannot outlive the callback:
    /// ```compile_fail
    /// use livegraph::GraphBuilder;
    /// let mut builder = GraphBuilder::new();
    /// let text = builder.resource("text", String::from("hello"));
    /// let graph = builder.build().unwrap();
    /// let escaped: &String = graph.with_snapshot(|snapshot| snapshot.get(text).unwrap());
    /// println!("{escaped}");
    /// ```
    #[inline]
    pub fn with_snapshot<R>(&self, read: impl FnOnce(&Snapshot) -> R) -> R {
        let snapshot = self.current.load();
        read(&snapshot)
    }

    /// Start a batch of resource replacements.
    pub fn changes(&self) -> Changes {
        Changes {
            graph: self.id,
            nodes: self.nodes.clone(),
            expected_version: None,
            error: None,
            values: BTreeMap::new(),
        }
    }
    /// Prepare replacements using values read from this snapshot.
    ///
    /// Commit rejects the whole batch with [`Error::Conflict`] if any intervening
    /// commit advanced the graph. On conflict, acquire a fresh snapshot and recompute
    /// replacements; do not retry the same stale values with an unchecked batch.
    pub fn changes_from(&self, snapshot: &Snapshot) -> Result<Changes> {
        if snapshot.graph != self.id {
            return Err(Error::ForeignGraph);
        }
        let mut changes = self.changes();
        changes.expected_version = Some(snapshot.version);
        Ok(changes)
    }

    fn populate<const ALL: bool>(
        &self,
        candidate: &mut Snapshot,
        replacements: BTreeMap<usize, Value>,
        pending: &[usize],
    ) -> Result<()> {
        for (i, value) in replacements {
            candidate.values.write::<ALL>(i, value);
        }
        for &position in pending {
            let i = self.order[position];
            if let Kind::Derived(f) = &self.nodes[i].kind {
                let value = evaluate(&self.nodes[i], f, candidate)?;
                candidate.values.write::<ALL>(i, value);
            }
        }
        Ok(())
    }

    /// Recompute affected descendants in dependency order, validate, and publish once.
    ///
    /// Errors and callback unwinds preserve the previous snapshot. Empty batches are
    /// no-ops after checking any expected version. Equal replacements still invalidate descendants (no equality pruning).
    /// Same-thread recursive commits return [`Error::ReentrantCommit`]. Callbacks
    /// must not wait for other threads to commit to this graph.
    pub fn commit(&self, changes: Changes) -> Result<Arc<Snapshot>> {
        if changes.graph != self.id {
            return Err(Error::ForeignGraph);
        }
        if let Some(error) = changes.error {
            return Err(error);
        }
        // These owners outlive the lock scope, including on error/unwind, so payload
        // cleanup cannot run while the writer mutex is held.
        let mut old_owner = None;
        let mut candidate_owner = None;
        let outcome = (|| {
            let _active = ActiveCommit::enter(self.id)?;
            let mut writer = self.writer.lock().map_err(|_| Error::WriterPoisoned)?;
            let old = old_owner.insert(self.current.load_full());
            if let Some(expected) = changes.expected_version
                && expected != old.version
            {
                return Err(Error::Conflict {
                    expected,
                    actual: old.version,
                });
            }
            if changes.values.is_empty() {
                return Ok(old.clone());
            }
            for &i in changes.values.keys() {
                let node = self.nodes.get(i).ok_or(Error::UnknownNode(i))?;
                if !matches!(node.kind, Kind::Resource(_)) {
                    return Err(Error::NotResource(node.name.clone()));
                }
            }
            let version = old.version.checked_add(1).ok_or(Error::VersionExhausted)?;
            writer.affected(
                changes.values.keys().copied(),
                &self.outgoing,
                &self.rank,
                &self.order,
            );
            let candidate = candidate_owner.insert(Snapshot {
                graph: self.id,
                version,
                values: if writer.pending.len() == self.nodes.len() {
                    Values::empty(self.nodes.len())
                } else {
                    old.values
                        .fork(writer.pending.iter().map(|&position| self.order[position]))
                },
            });
            if writer.pending.len() == self.nodes.len() {
                self.populate::<true>(candidate, changes.values, &writer.pending)?;
            } else {
                self.populate::<false>(candidate, changes.values, &writer.pending)?;
            }
            validate_all(&self.validators, candidate)?;
            let published = Arc::new(candidate_owner.take().expect("candidate prepared"));
            self.current.store(published.clone());
            Ok(published)
        })();
        drop(candidate_owner);
        drop(old_owner);
        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_version_does_not_publish() -> Result<()> {
        let mut b = GraphBuilder::new();
        let x = b.resource("x", 1);
        let g = b.build()?;
        g.current.store(Arc::new(Snapshot {
            graph: g.id,
            version: u64::MAX,
            values: g.snapshot().values.clone(),
        }));
        let old = g.snapshot();
        let mut changes = g.changes();
        changes.set(x, 2)?;
        assert!(matches!(g.commit(changes), Err(Error::VersionExhausted)));
        assert!(Arc::ptr_eq(&old, &g.snapshot()));
        Ok(())
    }

    #[test]
    fn obsolete_payload_is_released_after_last_snapshot() -> Result<()> {
        let mut b = GraphBuilder::new();
        let payload = Arc::new(String::from("old"));
        let weak = Arc::downgrade(&payload);
        let node = b.resource("payload", payload);
        let g = b.build()?;
        let pinned = g.snapshot();
        let mut changes = g.changes();
        changes.set(node, Arc::new(String::from("new")))?;
        g.commit(changes)?;
        assert!(weak.upgrade().is_some());
        drop(pinned);
        assert!(weak.upgrade().is_none());
        Ok(())
    }

    #[test]
    fn deep_graph_uses_iterative_scheduling() -> Result<()> {
        let mut b = GraphBuilder::new();
        let root = b.resource("root", 0_usize);
        let mut last = root;
        for i in 0..4096 {
            let previous = last;
            last = b.derived(format!("step {i}"), [previous.dep()], move |cx| {
                Ok(cx.get(previous)? + 1)
            })?;
        }
        let g = b.build()?;
        let mut changes = g.changes();
        changes.set(root, 10)?;
        assert_eq!(*g.commit(changes)?.get(last)?, 4106);
        Ok(())
    }
    #[test]
    fn retired_and_rejected_payloads_are_dropped_after_unlocking() -> Result<()> {
        use std::sync::{OnceLock, Weak, atomic::AtomicUsize};
        struct Probe {
            graph: Arc<OnceLock<Weak<LiveGraph>>>,
            drops: Arc<AtomicUsize>,
        }
        impl Drop for Probe {
            fn drop(&mut self) {
                if let Some(graph) = self.graph.get().and_then(Weak::upgrade) {
                    assert!(
                        graph.writer.try_lock().is_ok(),
                        "payload dropped with writer locked"
                    );
                    // The same-thread commit marker must also be cleared before cleanup.
                    assert!(graph.commit(graph.changes()).is_ok());
                    self.drops.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
        let holder = Arc::new(OnceLock::new());
        let drops = Arc::new(AtomicUsize::new(0));
        let probe = || Probe {
            graph: holder.clone(),
            drops: drops.clone(),
        };
        let mut b = GraphBuilder::new();
        let value = b.resource("probe", probe());
        let reject = b.resource("reject", false);
        let fail_build = b.resource("fail build", false);
        b.derived("builder", [fail_build.dep()], move |cx| {
            if *cx.get(fail_build)? {
                Err("build failed".into())
            } else {
                Ok(())
            }
        })?;
        b.validate("reject", move |s| {
            if *s.get(reject)? {
                Err("rejected".into())
            } else {
                Ok(())
            }
        });
        let graph = Arc::new(b.build()?);
        holder.set(Arc::downgrade(&graph)).unwrap();
        let mut changes = graph.changes();
        changes.set(value, probe())?;
        graph.commit(changes)?;
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        let old = graph.snapshot();
        let mut changes = graph.changes();
        changes.set(value, probe())?.set(reject, true)?;
        assert!(matches!(
            graph.commit(changes),
            Err(Error::Validation { .. })
        ));
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert!(Arc::ptr_eq(&old, &graph.snapshot()));
        let mut stale = graph.changes_from(&old)?;
        stale.set(value, probe())?;
        let mut advance = graph.changes();
        advance.set(reject, false)?;
        let current = graph.commit(advance)?;
        assert!(matches!(graph.commit(stale), Err(Error::Conflict { .. })));
        assert_eq!(drops.load(Ordering::SeqCst), 3);
        assert!(Arc::ptr_eq(&current, &graph.snapshot()));
        let mut failure = graph.changes();
        failure.set(value, probe())?.set(fail_build, true)?;
        assert!(matches!(graph.commit(failure), Err(Error::Build { .. })));
        assert_eq!(drops.load(Ordering::SeqCst), 4);
        assert!(Arc::ptr_eq(&current, &graph.snapshot()));
        Ok(())
    }
    #[test]
    fn snapshots_copy_only_touched_chunks_including_partial_tail() -> Result<()> {
        let mut builder = GraphBuilder::new();
        let nodes: Vec<_> = (0..CHUNK_SIZE * 4 + 5)
            .map(|i| builder.resource(format!("node-{i}"), i))
            .collect();
        let graph = builder.build()?;
        let old = graph.snapshot();
        let touched = [CHUNK_SIZE - 2, CHUNK_SIZE - 1, CHUNK_SIZE, nodes.len() - 1];
        let mut changes = graph.changes();
        for &i in &touched {
            changes.set(nodes[i], 1000 + i)?;
        }
        let new = graph.commit(changes)?;
        for i in 0..old.values.chunks.len() {
            assert_eq!(
                Arc::ptr_eq(&old.values.chunks[i], &new.values.chunks[i]),
                ![0, 1, 4].contains(&i)
            );
        }
        for (i, &node) in nodes.iter().enumerate() {
            assert_eq!(*old.get(node)?, i);
            assert_eq!(
                *new.get(node)?,
                if touched.contains(&i) { 1000 + i } else { i }
            );
        }
        assert!(new.values.get(nodes.len()).is_none());
        Ok(())
    }

    #[test]
    fn scheduler_epoch_wrap_clears_stale_marks_and_keeps_order() -> Result<()> {
        let mut builder = GraphBuilder::new();
        let sum = builder.reserve::<u64>("sum");
        let a = builder.resource("a", 1_u64);
        let b = builder.resource("b", 2_u64);
        builder.define_derived(sum, [a.dep(), b.dep()], move |cx| {
            Ok(cx.get(a)? + cx.get(b)?)
        })?;
        let graph = builder.build()?;
        {
            let mut writer = graph.writer.lock().unwrap();
            writer.epoch = u64::MAX;
            writer.seen.fill(1);
        }
        let mut changes = graph.changes();
        changes.set(a, 4)?;
        assert_eq!(*graph.commit(changes)?.get(sum)?, 6);
        let mut changes = graph.changes();
        changes.set(b, 7)?;
        assert_eq!(*graph.commit(changes)?.get(sum)?, 11);
        assert_eq!(graph.writer.lock().unwrap().pending.len(), 2);
        Ok(())
    }
    #[test]
    fn scheduler_selects_once_from_staged_roots_and_completes_reachability() {
        let order = [4, 0, 7, 5, 1, 2, 3, 8, 9, 6, 10, 11, 12, 13, 14, 15];
        let mut rank = vec![0; order.len()];
        for (i, &node) in order.iter().enumerate() {
            rank[node] = i;
        }
        let mut outgoing = vec![vec![]; order.len()];
        outgoing[0] = vec![5];
        outgoing[5] = vec![9];
        let mut scheduler = Scheduler::new(order.len());
        assert_eq!(
            scheduler.affected([0].into_iter(), &outgoing, &rank, &order),
            UpdateStrategy::Sparse
        );
        assert_eq!(scheduler.pending, vec![rank[0], rank[5], rank[9]]);
        outgoing[9] = vec![14];
        outgoing[14] = vec![15];
        assert_eq!(
            scheduler.affected([0].into_iter(), &outgoing, &rank, &order),
            UpdateStrategy::Sparse
        );
        assert_eq!(
            scheduler.pending,
            vec![rank[0], rank[5], rank[9], rank[14], rank[15]]
        );
        assert_eq!(
            scheduler.affected([4].into_iter(), &outgoing, &rank, &order),
            UpdateStrategy::Sparse
        );
        assert_eq!(scheduler.pending, vec![rank[4]]);
        assert_eq!(
            scheduler.affected([0, 1, 2, 3].into_iter(), &outgoing, &rank, &order),
            UpdateStrategy::Dense
        );
        assert_eq!(scheduler.pending, vec![1, 3, 4, 5, 6, 8, 14, 15]);
    }

    #[test]
    fn adaptive_schedules_match_independent_reachability_on_permuted_dags() {
        const N: usize = 64;
        for seed in 1..=12_u64 {
            let mut random = seed;
            let mut next = || {
                random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
                random
            };
            let mut order: Vec<_> = (0..N).collect();
            for i in (1..N).rev() {
                order.swap(i, (next() as usize) % (i + 1));
            }
            let mut rank = vec![0; N];
            for (i, &node) in order.iter().enumerate() {
                rank[node] = i;
            }
            let mut outgoing = vec![vec![]; N];
            for i in 0..N {
                for j in i + 1..N {
                    if next().is_multiple_of(23) {
                        outgoing[order[i]].push(order[j]);
                    }
                }
            }
            let mut scheduler = Scheduler::new(N);
            for count in [1, 3, 15, 16, 40, 1] {
                let roots: Vec<_> = (0..count).map(|i| order[(i * 7) % N]).collect();
                let mut seen = [false; N];
                let mut stack = roots.clone();
                while let Some(node) = stack.pop() {
                    if !seen[node] {
                        seen[node] = true;
                        stack.extend(&outgoing[node]);
                    }
                }
                let expected: Vec<_> = order
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &n)| seen[n].then_some(i))
                    .collect();
                let strategy = scheduler.affected(roots.into_iter(), &outgoing, &rank, &order);
                assert_eq!(scheduler.pending, expected, "seed {seed}, count {count}");
                assert_eq!(strategy == UpdateStrategy::Dense, count >= N / 4);
            }
        }
    }
    #[test]
    fn broadcast_roots_keep_one_reachable_traversal_instead_of_restarting() {
        let order: Vec<_> = (0..1024).collect();
        let mut outgoing = vec![vec![]; order.len()];
        outgoing[0] = (1..1024).collect();
        let mut scheduler = Scheduler::new(order.len());
        assert_eq!(
            scheduler.affected([0].into_iter(), &outgoing, &order, &order),
            UpdateStrategy::Sparse
        );
        assert_eq!(scheduler.pending, order);
    }
}
