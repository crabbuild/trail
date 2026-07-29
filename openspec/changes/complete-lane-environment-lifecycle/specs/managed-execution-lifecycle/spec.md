## ADDED Requirements

### Requirement: All execution surfaces use one managed lifecycle
Terminal agents, ACP sessions, tests, evals, and lane exec SHALL use the same ordered environment lifecycle.

#### Scenario: Managed execution succeeds
- **WHEN** any supported execution surface launches a command
- **THEN** Trail spawns or resolves the lane, discovers and plans adapters, synchronizes all components, reconciles runtime resources, mounts the view, executes, checkpoints source changes, disposes execution-owned artifacts, and unmounts in that order

#### Scenario: Preparation fails
- **WHEN** discovery, planning, synchronization, reconciliation, or mounting fails
- **THEN** Trail does not launch the command and finalizes every resource acquired by earlier phases

#### Scenario: Command fails
- **WHEN** the launched command exits unsuccessfully or is interrupted
- **THEN** Trail preserves the command outcome, performs the configured source checkpoint policy, and finalizes all execution-owned resources

### Requirement: Managed execution checkpoints only durable source
Trail MUST exclude dependency, generated, scratch, secret, and internal paths from execution checkpoints.

#### Scenario: Source and generated files both change
- **WHEN** an execution changes a source file and a generated artifact
- **THEN** the source change is eligible for the lane checkpoint and the generated artifact is absent from the recorded change

### Requirement: Finalization failures remain visible
Trail SHALL report command, checkpoint, runtime cleanup, artifact disposal, and unmount outcomes without one failure hiding another.

#### Scenario: Command and cleanup both fail
- **WHEN** a command exits nonzero and cleanup also fails
- **THEN** the report retains the command exit as the primary outcome and includes the cleanup failure and resumable cleanup identity

### Requirement: Lifecycle receipts are auditable
Trail SHALL emit durable phase receipts that identify the lane, view, environment generation, execution surface, command fingerprint, checkpoint result, and disposal result.

#### Scenario: Inspect completed execution
- **WHEN** an operator inspects lane events after execution
- **THEN** the ordered lifecycle phases and their outcomes can be reconstructed without retaining disposable artifact contents
