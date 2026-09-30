use crate::{
    CommandEnvironmentSpec, ContractId, EventId, HarnessId, NativeContext, RawInvocation,
    SnapshotId,
};

/// Cross-harness lifecycle category. This is never an exact protocol identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EventCategory {
    /// Session lifecycle activity.
    Session,
    /// User-prompt lifecycle activity.
    Prompt,
    /// Tool invocation activity.
    Tool,
    /// Model request or response activity.
    Model,
    /// Subagent lifecycle activity.
    Agent,
    /// Context management activity.
    Context,
    /// Worktree lifecycle activity.
    Worktree,
    /// Activity without a stable cross-harness category.
    Other,
}

/// Native mechanism used to invoke a hook handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandlerKind {
    /// A local process using stdin, stdout, stderr, and an exit code.
    Command,
    /// An HTTP callback.
    Http,
    /// An MCP tool invocation.
    McpTool,
    /// A prompt expanded and submitted to a model.
    Prompt,
    /// An agent handler.
    Agent,
}

/// Fully materialized process result. Bytes are retained exactly: empty stdout
/// is distinct from JSON null and text is not normalized with a newline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessEmission {
    contract: ContractId,
    binding: HandlerKind,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    exit_code: u8,
}

impl ProcessEmission {
    /// Protocol-author constructor for an empty successful command response.
    pub fn command_empty(contract: ContractId) -> Self {
        Self::command_unchecked(contract, Vec::new(), Vec::new(), 0)
    }

    /// Protocol-author constructor for a successful JSON command response.
    pub fn command_json<T: serde::Serialize>(
        contract: ContractId,
        value: &T,
    ) -> crate::Result<Self> {
        Ok(Self::command_unchecked(
            contract,
            serde_json::to_vec(value)?,
            Vec::new(),
            0,
        ))
    }

    /// Protocol-author constructor for exact successful text/opaque bytes.
    pub fn command_text(contract: ContractId, value: impl Into<Vec<u8>>) -> Self {
        Self::command_unchecked(contract, value.into(), Vec::new(), 0)
    }

    /// Protocol-author constructor for a stderr-only command outcome.
    pub fn command_stderr(
        contract: ContractId,
        message: impl Into<Vec<u8>>,
        exit_code: u8,
    ) -> crate::Result<Self> {
        if exit_code == 0 {
            return Err(crate::HookkitError::InvalidProcessEmission(
                "stderr-only protocol outcome requires a nonzero exit code",
            ));
        }
        Ok(Self::command_unchecked(
            contract,
            Vec::new(),
            message.into(),
            exit_code,
        ))
    }

    /// Protocol-author constructor for outcomes whose contract requires a
    /// non-empty UTF-8 stderr message.
    pub fn command_required_stderr(
        contract: ContractId,
        message: impl Into<String>,
        exit_code: u8,
    ) -> crate::Result<Self> {
        let message = message.into();
        if message.is_empty() {
            return Err(crate::HookkitError::InvalidProcessEmission(
                "protocol outcome requires non-empty stderr",
            ));
        }
        Self::command_stderr(contract, message, exit_code)
    }

    /// Explicit unchecked escape hatch for protocol authors. Application event
    /// handlers cannot return `ProcessEmission` through the typed APIs.
    pub fn command_unchecked(
        contract: ContractId,
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        exit_code: u8,
    ) -> Self {
        Self {
            contract,
            binding: HandlerKind::Command,
            stdout,
            stderr,
            exit_code,
        }
    }

    /// Returns the event/binding contract used to construct this result.
    pub fn contract(&self) -> ContractId {
        self.contract
    }

    /// Returns the native handler mechanism for this result.
    pub fn binding(&self) -> HandlerKind {
        self.binding
    }

    /// Returns stdout exactly as it should be written, without newline changes.
    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    /// Returns stderr exactly as it should be written, without newline changes.
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    /// Returns the process exit code.
    pub fn exit_code(&self) -> u8 {
        self.exit_code
    }
}

/// Exact event contract. The associated output prevents one event's response
/// from being returned by another event's typed runner.
pub trait EventSpec {
    /// Exact native input type accepted by the event.
    type Input;
    /// Native command-hook environment parsed alongside the input.
    type CommandEnvironment: CommandEnvironmentSpec;
    /// Exact output type accepted by the event's command binding.
    type CommandOutput;

    /// Harness that owns this event.
    const HARNESS: HarnessId;
    /// Immutable catalog snapshot in which the contract was defined.
    const SNAPSHOT: SnapshotId;
    /// Exact harness-scoped wire event identity.
    const EVENT: EventId;
    /// Approximate cross-harness lifecycle category.
    const CATEGORY: EventCategory;
    /// Immutable identity of this input/output command contract.
    const CONTRACT: ContractId;

    /// Parses the exact native input from a lossless invocation.
    fn parse(invocation: &RawInvocation) -> crate::Result<Self::Input>;
    /// Encodes the typed command output into exact process bytes and status.
    fn emit(output: Self::CommandOutput) -> crate::Result<ProcessEmission>;

    /// Validate redundant native-input and command-environment state before a
    /// handler is called. Harness adapters can override this for fields that
    /// are not part of [`NativeContext`].
    fn validate_command_environment(
        input: &Self::Input,
        environment: &Self::CommandEnvironment,
    ) -> crate::Result<()> {
        environment.validate_context(&Self::EVENT, &Self::context(input))
    }

    /// Extract only fields the exact native input contract supplies.
    fn context(_input: &Self::Input) -> NativeContext {
        NativeContext::default()
    }
}

/// Machine-enumerable implementation declaration. Construction requires an
/// actual [`EventSpec`], preventing catalog-only events from being marked as
/// implemented by a boolean literal.
#[derive(Debug, Clone)]
pub struct NativeEventDescriptor {
    contract: ContractId,
    event: EventId,
    snapshot: SnapshotId,
    bindings: Vec<HandlerKind>,
    conformance_cases: Vec<&'static str>,
}

impl NativeEventDescriptor {
    /// Declares the command binding implemented by `E` and the conformance
    /// fixture names exercised for it.
    ///
    /// Fixture names are retained verbatim and are not validated here.
    pub fn command<E: EventSpec>(conformance_cases: &[&'static str]) -> Self {
        Self {
            contract: E::CONTRACT,
            event: E::EVENT,
            snapshot: E::SNAPSHOT,
            bindings: vec![HandlerKind::Command],
            conformance_cases: conformance_cases.to_vec(),
        }
    }

    /// Returns the implemented event/binding contract.
    pub fn contract(&self) -> ContractId {
        self.contract
    }

    /// Returns the exact implemented event.
    pub fn event(&self) -> &EventId {
        &self.event
    }

    /// Returns the catalog snapshot implemented by the adapter.
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }

    /// Reports whether the descriptor has a native input implementation.
    ///
    /// A [`NativeEventDescriptor`] can only be constructed from an
    /// [`EventSpec`], so this currently always returns `true`.
    pub fn native_input(&self) -> bool {
        true
    }

    /// Reports whether the descriptor has a native output implementation.
    ///
    /// A [`NativeEventDescriptor`] can only be constructed from an
    /// [`EventSpec`], so this currently always returns `true`.
    pub fn native_output(&self) -> bool {
        true
    }

    /// Returns all implemented native handler mechanisms.
    pub fn bindings(&self) -> &[HandlerKind] {
        &self.bindings
    }

    /// Returns the declared conformance fixture names.
    pub fn conformance_cases(&self) -> &[&'static str] {
        &self.conformance_cases
    }
}

/// Confidence with which payload shape can identify a native event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IdentificationStrength {
    /// A documented discriminator uniquely names the event.
    Definitive,
    /// Shape alone is sufficient after known overlaps are excluded.
    SoundShape,
    /// Shape is useful evidence but cannot independently authorize execution.
    WeakShape,
    /// The event intentionally shares its shape with one or more other events.
    Ambiguous,
    /// The event cannot be identified from payload data available to the hook.
    Impossible,
}

/// Lightweight production identification metadata paired with the exact native
/// parser for an implemented event.
#[derive(Clone)]
pub struct IdentificationDescriptor {
    event: EventId,
    contract: ContractId,
    snapshot: SnapshotId,
    strength: IdentificationStrength,
    discriminator: Option<(&'static str, &'static str)>,
    overlaps: &'static [&'static str],
    validate: Option<fn(&RawInvocation) -> crate::Result<()>>,
}

fn validate_event<E: EventSpec>(raw: &RawInvocation) -> crate::Result<()> {
    E::parse(raw).map(|_| ())
}

impl std::fmt::Debug for IdentificationDescriptor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("IdentificationDescriptor")
            .field("event", &self.event)
            .field("contract", &self.contract)
            .field("snapshot", &self.snapshot)
            .field("strength", &self.strength)
            .field("discriminator", &self.discriminator)
            .field("overlaps", &self.overlaps)
            .field("native_parser", &self.validate.is_some())
            .finish()
    }
}

impl IdentificationDescriptor {
    /// Describes an implemented event identified by an exact JSON pointer and
    /// string value.
    pub fn definitive<E: EventSpec>(pointer: &'static str, value: &'static str) -> Self {
        Self::new::<E>(
            IdentificationStrength::Definitive,
            Some((pointer, value)),
            &[],
        )
    }

    /// Describes an implemented event whose validated payload shape is safe
    /// for identification after considering `overlaps`.
    pub fn sound_shape<E: EventSpec>(overlaps: &'static [&'static str]) -> Self {
        Self::new::<E>(IdentificationStrength::SoundShape, None, overlaps)
    }

    /// Describes an implemented event whose shape is useful only as supporting
    /// evidence because it is not sufficient for automatic selection.
    pub fn weak_shape<E: EventSpec>(overlaps: &'static [&'static str]) -> Self {
        Self::new::<E>(IdentificationStrength::WeakShape, None, overlaps)
    }

    /// Describes an implemented event that cannot be distinguished from the
    /// named overlapping events by shape alone.
    pub fn ambiguous<E: EventSpec>(overlaps: &'static [&'static str]) -> Self {
        Self::new::<E>(IdentificationStrength::Ambiguous, None, overlaps)
    }

    /// Describes an implemented event with no usable in-band identification
    /// evidence. A validated caller hint is required to select it.
    pub fn impossible<E: EventSpec>(overlaps: &'static [&'static str]) -> Self {
        Self::new::<E>(IdentificationStrength::Impossible, None, overlaps)
    }

    fn new<E: EventSpec>(
        strength: IdentificationStrength,
        discriminator: Option<(&'static str, &'static str)>,
        overlaps: &'static [&'static str],
    ) -> Self {
        Self {
            event: E::EVENT,
            contract: E::CONTRACT,
            snapshot: E::SNAPSHOT,
            strength,
            discriminator,
            overlaps,
            validate: Some(validate_event::<E>),
        }
    }

    /// Catalog identity metadata for an event whose native parser is not yet
    /// implemented. This is detector/resolution evidence only and can never
    /// authorize execution.
    pub fn catalog_definitive(
        event: EventId,
        snapshot: SnapshotId,
        contract: ContractId,
        pointer: &'static str,
        value: &'static str,
    ) -> Self {
        Self {
            event,
            contract,
            snapshot,
            strength: IdentificationStrength::Definitive,
            discriminator: Some((pointer, value)),
            overlaps: &[],
            validate: None,
        }
    }

    /// Returns the exact event described by this identification rule.
    pub fn event(&self) -> &EventId {
        &self.event
    }

    /// Returns the event/binding contract associated with the rule.
    pub fn contract(&self) -> ContractId {
        self.contract
    }

    /// Returns the catalog snapshot associated with the rule.
    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }

    /// Returns the rule's identification confidence.
    pub fn strength(&self) -> IdentificationStrength {
        self.strength
    }

    /// Returns the authoritative `(JSON pointer, string value)` discriminator,
    /// if the event has one.
    pub fn discriminator(&self) -> Option<(&'static str, &'static str)> {
        self.discriminator
    }

    /// Returns harness-native names of events known to overlap this shape.
    pub fn overlaps(&self) -> &'static [&'static str] {
        self.overlaps
    }

    /// Returns [`EventId`] values for the harness-native overlap names.
    pub fn overlap_events(&self) -> Vec<EventId> {
        self.overlaps
            .iter()
            .map(|name| EventId::builtin(self.event.harness().clone(), name))
            .collect()
    }

    /// Reports whether this descriptor was constructed from an implemented
    /// [`EventSpec`] and can therefore validate native input.
    pub fn has_native_parser(&self) -> bool {
        self.validate.is_some()
    }

    /// Validates `raw` with the event's exact native parser.
    ///
    /// Catalog-only descriptors return
    /// [`crate::HookkitError::NativeParserUnavailable`].
    pub fn validate(&self, raw: &RawInvocation) -> crate::Result<()> {
        self.validate
            .ok_or_else(|| crate::HookkitError::NativeParserUnavailable {
                event: self.event.clone(),
            })?(raw)
    }
}

/// Harness-scoped selector used by compile-time selected-harness execution.
pub trait EventSelector {
    /// Returns the exact event selected for dynamic dispatch.
    fn event_id(&self) -> EventId;
}

/// A selected-harness dynamic contract. Input and output enums must expose their
/// exact event arm so the runtime can reject a mismatch before emission.
pub trait HarnessSpec {
    /// Sum type containing every implemented native input for the harness.
    type AnyInput;
    /// Harness-wide native command environment.
    type CommandEnvironment: CommandEnvironmentSpec;
    /// Sum type containing every implemented command output for the harness.
    type AnyCommandOutput;
    /// Application-facing selector for an exact event.
    type EventSelector: EventSelector;

    /// Stable harness identity.
    const ID: HarnessId;
    /// Immutable catalog snapshot implemented by this adapter.
    const SNAPSHOT: SnapshotId;

    /// Returns identification metadata for implemented and catalog-only events.
    fn identification_descriptors() -> Vec<IdentificationDescriptor>;
    /// Decodes `raw` as the explicitly selected `event`.
    fn decode(event: &EventId, raw: &RawInvocation) -> crate::Result<Self::AnyInput>;
    /// Returns the exact event represented by a decoded input arm.
    fn input_event(input: &Self::AnyInput) -> EventId;
    /// Validates redundant native-input and command-environment state before
    /// invoking an application handler.
    fn validate_command_environment(
        input: &Self::AnyInput,
        environment: &Self::CommandEnvironment,
    ) -> crate::Result<()> {
        let event = Self::input_event(input);
        environment.validate_context(&event, &Self::context(input))
    }
    /// Returns the exact event represented by a dynamic output arm.
    fn output_event(output: &Self::AnyCommandOutput) -> EventId;
    /// Checked dynamic encoding. The triggering event is mandatory so a caller
    /// cannot emit an arbitrary output arm without the same agreement check the
    /// selected runtime performs.
    fn encode_command(
        expected_event: &EventId,
        output: Self::AnyCommandOutput,
    ) -> crate::Result<ProcessEmission> {
        let output_event = Self::output_event(&output);
        if &output_event != expected_event {
            return Err(crate::HookkitError::OutputEventMismatch {
                input: expected_event.clone(),
                output: output_event,
            });
        }
        Self::encode_command_unchecked(output)
    }

    /// Protocol-author escape hatch. Application dispatch should call
    /// [`HarnessSpec::encode_command`] with its triggering event.
    fn encode_command_unchecked(output: Self::AnyCommandOutput) -> crate::Result<ProcessEmission>;
    /// Extracts only context fields explicitly supplied by the selected native
    /// input arm.
    fn context(input: &Self::AnyInput) -> NativeContext;
}
