use crate::{
    CommandEnvironmentSpec, ContractId, EventId, HarnessId, NativeContext, RawInvocation,
    SnapshotId,
};

/// Cross-harness lifecycle category. This is never an exact protocol identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventCategory {
    Session,
    Prompt,
    Tool,
    Model,
    Agent,
    Context,
    Worktree,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandlerKind {
    Command,
    Http,
    McpTool,
    Prompt,
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

    pub fn contract(&self) -> ContractId {
        self.contract
    }

    pub fn binding(&self) -> HandlerKind {
        self.binding
    }

    pub fn stdout(&self) -> &[u8] {
        &self.stdout
    }

    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }

    pub fn exit_code(&self) -> u8 {
        self.exit_code
    }
}

/// Exact event contract. The associated output prevents one event's response
/// from being returned by another event's typed runner.
pub trait EventSpec {
    type Input;
    type CommandEnvironment: CommandEnvironmentSpec;
    type CommandOutput;

    const HARNESS: HarnessId;
    const SNAPSHOT: SnapshotId;
    const EVENT: EventId;
    const CATEGORY: EventCategory;
    const CONTRACT: ContractId;

    fn parse(invocation: &RawInvocation) -> crate::Result<Self::Input>;
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
    pub fn command<E: EventSpec>(conformance_cases: &[&'static str]) -> Self {
        Self {
            contract: E::CONTRACT,
            event: E::EVENT,
            snapshot: E::SNAPSHOT,
            bindings: vec![HandlerKind::Command],
            conformance_cases: conformance_cases.to_vec(),
        }
    }

    pub fn contract(&self) -> ContractId {
        self.contract
    }

    pub fn event(&self) -> &EventId {
        &self.event
    }

    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }

    pub fn native_input(&self) -> bool {
        true
    }

    pub fn native_output(&self) -> bool {
        true
    }

    pub fn bindings(&self) -> &[HandlerKind] {
        &self.bindings
    }

    pub fn conformance_cases(&self) -> &[&'static str] {
        &self.conformance_cases
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentificationStrength {
    Definitive,
    SoundShape,
    WeakShape,
    Ambiguous,
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

impl IdentificationDescriptor {
    pub fn definitive<E: EventSpec>(pointer: &'static str, value: &'static str) -> Self {
        Self::new::<E>(
            IdentificationStrength::Definitive,
            Some((pointer, value)),
            &[],
        )
    }

    pub fn sound_shape<E: EventSpec>(overlaps: &'static [&'static str]) -> Self {
        Self::new::<E>(IdentificationStrength::SoundShape, None, overlaps)
    }

    pub fn weak_shape<E: EventSpec>(overlaps: &'static [&'static str]) -> Self {
        Self::new::<E>(IdentificationStrength::WeakShape, None, overlaps)
    }

    pub fn ambiguous<E: EventSpec>(overlaps: &'static [&'static str]) -> Self {
        Self::new::<E>(IdentificationStrength::Ambiguous, None, overlaps)
    }

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

    pub fn event(&self) -> &EventId {
        &self.event
    }

    pub fn contract(&self) -> ContractId {
        self.contract
    }

    pub fn snapshot(&self) -> SnapshotId {
        self.snapshot
    }

    pub fn strength(&self) -> IdentificationStrength {
        self.strength
    }

    pub fn discriminator(&self) -> Option<(&'static str, &'static str)> {
        self.discriminator
    }

    pub fn overlaps(&self) -> &'static [&'static str] {
        self.overlaps
    }

    pub fn overlap_events(&self) -> Vec<EventId> {
        self.overlaps
            .iter()
            .map(|name| EventId::builtin(self.event.harness().clone(), name))
            .collect()
    }

    pub fn has_native_parser(&self) -> bool {
        self.validate.is_some()
    }

    pub fn validate(&self, raw: &RawInvocation) -> crate::Result<()> {
        self.validate
            .ok_or_else(|| crate::HookkitError::NativeParserUnavailable {
                event: self.event.clone(),
            })?(raw)
    }
}

/// Harness-scoped selector used by compile-time selected-harness execution.
pub trait EventSelector {
    fn event_id(&self) -> EventId;
}

/// A selected-harness dynamic contract. Input and output enums must expose their
/// exact event arm so the runtime can reject a mismatch before emission.
pub trait HarnessSpec {
    type AnyInput;
    type CommandEnvironment: CommandEnvironmentSpec;
    type AnyCommandOutput;
    type EventSelector: EventSelector;

    const ID: HarnessId;
    const SNAPSHOT: SnapshotId;

    fn identification_descriptors() -> Vec<IdentificationDescriptor>;
    fn decode(event: &EventId, raw: &RawInvocation) -> crate::Result<Self::AnyInput>;
    fn input_event(input: &Self::AnyInput) -> EventId;
    fn validate_command_environment(
        input: &Self::AnyInput,
        environment: &Self::CommandEnvironment,
    ) -> crate::Result<()> {
        let event = Self::input_event(input);
        environment.validate_context(&event, &Self::context(input))
    }
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
    fn context(input: &Self::AnyInput) -> NativeContext;
}
