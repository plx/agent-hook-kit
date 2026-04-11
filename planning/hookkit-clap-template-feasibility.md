
# Hookkit Templates
## Feasibility of Reusable clap-Based CLI Templates

Version: draft 0.1

## 1. Goal

The stretch-goal library is not the hook parser/emitter itself. It is a companion layer that makes it fast to build many small hook executables.

Desired ergonomics:

- define a hook tool once,
- reuse the same parsing/emission plumbing,
- expose a normal `clap` CLI,
- provide a minimal amount of custom code per executable.

## 2. Constraint that matters most

A runtime plugin architecture built directly on `dyn clap::Subcommand` or `dyn clap::Args` is not viable.

`clap::Subcommand`, `clap::Args`, and `clap::CommandFactory` are not dyn-compatible / not object-safe. That means a design centered on storing heterogeneous subcommands behind trait objects like:

```rust
Vec<Box<dyn clap::Subcommand>>
```

is the wrong direction.

This single fact should drive the design.

## 3. What *is* feasible

Three approaches are feasible.

## 3.1 Static compile-time composition (recommended primary path)

This is the most ergonomic path for a Rust workspace where hook tools are known at compile time.

### Shape

Each hook tool provides:

- an args type deriving `clap::Args`,
- a pure handler implementation,
- metadata constants like name/about.

A macro or codegen helper creates:

- the top-level enum of subcommands,
- the dispatch glue,
- the root `Parser`.

### Example shape

```rust
#[derive(clap::Args, Debug)]
pub struct TsLintArgs {
    #[arg(long)]
    autofix: bool,
}

pub struct TsLintHook;

impl HookCommand for TsLintHook {
    type Args = TsLintArgs;
    const NAME: &'static str = "ts-lint";
    const ABOUT: &'static str = "Post-tool TypeScript formatter/linter hook";

    fn run(
        args: &Self::Args,
        ctx: &HookCommandContext,
    ) -> hookkit_core::Result<HookExecution> {
        // business logic
    }
}
```

Then:

```rust
hookkit_templates::register_commands![
    TsLintHook,
    SwiftLintHook,
    RustFmtHook,
];
```

The macro generates the enum and dispatch.

### Why this is good

- strong typing,
- excellent `clap` UX,
- simple binaries,
- no object-safety fight,
- no `ArgMatches` manual parsing in most user code.

## 3.2 Builder/registry composition (recommended secondary path)

This is useful when the command set is assembled from library crates or feature flags and you want looser coupling.

### Shape

Define an object-safe trait of your own, not a `clap` trait:

```rust
pub trait CommandSpec: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn command(&self) -> clap::Command;
    fn run(
        &self,
        matches: &clap::ArgMatches,
        ctx: &HookCommandContext,
    ) -> hookkit_core::Result<HookExecution>;
}
```

Then the framework:

1. collects `Vec<Box<dyn CommandSpec>>`,
2. folds them into a root `clap::Command`,
3. parses matches,
4. dispatches by subcommand name.

### Why this is good

- object safe,
- runtime-extensible inside one compiled binary,
- compatible with feature-gated command registration,
- no need to force `clap`’s derive traits into a trait-object role they do not support.

### Tradeoff

The command author typically has to parse `ArgMatches` manually or use a helper conversion step.

## 3.3 Hybrid composition (recommended end state)

Support both:

- **static derive mode** for the nicest ergonomics,
- **registry mode** for advanced assembly.

The two modes can share the same underlying command execution trait.

## 4. Recommended architecture for `hookkit-templates`

## 4.1 Core traits

### Logic trait

```rust
pub trait HookLogic {
    type Input;
    type Output;

    fn handle(
        &self,
        input: Self::Input,
        ctx: &hookkit_runtime::RuntimeContext,
    ) -> hookkit_core::Result<Self::Output>;
}
```

### Template command trait

```rust
pub trait HookCommand {
    type Args: clap::Args + Send + Sync + 'static;

    const NAME: &'static str;
    const ABOUT: &'static str;

    fn harness() -> hookkit_core::Harness;

    fn parse_mode() -> ParseMode;

    fn run(
        args: &Self::Args,
        raw_ctx: &HookCommandContext,
    ) -> hookkit_core::Result<HookExecution>;
}
```

`ParseMode` might be:

```rust
pub enum ParseMode {
    Native,
    Common,
}
```

### Runtime-erased registry trait

```rust
pub trait CommandSpec: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn about(&self) -> &'static str;
    fn command(&self) -> clap::Command;
    fn run_matches(
        &self,
        matches: &clap::ArgMatches,
        ctx: &HookCommandContext,
    ) -> hookkit_core::Result<HookExecution>;
}
```

## 4.2 Command context

```rust
pub struct HookCommandContext {
    pub harness: hookkit_core::Harness,
    pub runtime: hookkit_runtime::RuntimeContext,
}
```

## 4.3 Execution result

```rust
pub struct HookExecution {
    pub output: hookkit_common::CommonHookOutput,
}
```

Or for native mode:

```rust
pub enum HookExecution {
    Native(NativeHookOutput),
    Common(CommonHookOutput),
}
```

## 5. Recommended user-facing patterns

## 5.1 Simplest generated binary

Generated binaries should support a small standard CLI:

```text
my-hook   --claude   post-tool-ts   --autofix   --profile mid-edit
```

or possibly:

```text
my-hook post-tool-ts --harness claude --autofix
```

I recommend a uniform top-level harness selector plus subcommands.

## 5.2 Suggested top-level CLI structure

```rust
#[derive(clap::Parser)]
struct Cli {
    #[arg(long, conflicts_with_all = ["codex", "gemini"])]
    claude: bool,
    #[arg(long, conflicts_with_all = ["claude", "gemini"])]
    codex: bool,
    #[arg(long, conflicts_with_all = ["claude", "codex"])]
    gemini: bool,

    #[command(subcommand)]
    command: Commands,
}
```

Then `Commands` is either:

- macro-generated enum for static mode, or
- a small hand-written wrapper in registry mode.

## 5.3 Good standard command categories

The templates crate should ship examples or helpers for commands like:

- `pre-tool-bash-policy`
- `post-tool-format-lint`
- `session-start-context`
- `stop-quality-gate`

These are examples, not baked-in assumptions.

## 6. Recommended implementation plan for templates

## Phase T1 — prove the builder/registry path

Start with the object-safe registry path because it is simpler to implement and validates the execution model.

Deliverables:

- `CommandSpec` trait
- root builder function
- dispatch logic
- one registry-driven sample binary

## Phase T2 — add derive-friendly adapters

Add helpers that let a command author define:

- a `#[derive(Args)]` type,
- a thin wrapper implementing `CommandSpec`.

This removes most manual `ArgMatches` work.

## Phase T3 — add static macro composition

Add a macro that generates:

- top-level enum,
- parser impl,
- dispatch boilerplate.

This becomes the nicest authoring experience for most cases.

## 7. API recommendation for the post-tool-use use case

For your likely high-volume hook tools, a particularly useful template is:

```rust
pub trait PostToolUseCommand {
    type Args: clap::Args;
    type Event;

    const NAME: &'static str;
    const ABOUT: &'static str;

    fn harness() -> hookkit_core::Harness;
    fn parse_event(
        input: hookkit_common::CommonHookInput,
    ) -> hookkit_core::Result<Self::Event>;

    fn handle(
        args: &Self::Args,
        event: Self::Event,
        ctx: &HookCommandContext,
    ) -> hookkit_core::Result<hookkit_common::CommonHookOutput>;
}
```

Then the framework provides the standard plumbing:

1. parse CLI args,
2. detect harness,
3. parse stdin into common/native input,
4. downcast to the expected event type,
5. call `handle`,
6. emit output.

That gives you the “drop in one method that handles parsed input -> structured output” feel you want, without trying to make `clap` traits themselves do something they were not designed to do.

## 8. Recommended conclusion

A reusable templates library is absolutely feasible in Rust.

What is **not** feasible is the most obvious “trait object over `clap::Subcommand`” design.

The best route is:

1. **implement the hook parser/emitter library first,**
2. **add an object-safe command registry second,**
3. **add macro-based static composition third.**

That sequence gets you something usable quickly and avoids painting the design into an object-safety corner.
