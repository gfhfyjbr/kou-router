# RTK product analysis and improvement plan

Date: 2026-06-11

Assumption: in this document, `RTK` means Rust Token Killer, the CLI proxy from
`rtk-ai/rtk`, not RTK GPS/real-time kinematics.

Sources checked:

- https://github.com/rtk-ai/rtk
- https://www.rtk-ai.app/
- https://github.com/rtk-ai/rtk/issues/545
- https://github.com/chopratejas/headroom

## Short summary

RTK is a local command-output compression layer for coding agents. It intercepts
shell commands, rewrites supported commands to `rtk ...`, runs the real command,
filters the output, and gives the model a shorter version.

The core idea is strong: most terminal output is progress bars, repeated logs,
successful tests, dependency noise, boilerplate, or huge lists. A model usually
does not need all of that. It needs status, exit code, root cause, locations, and
a way to ask for more.

The main product risk is also clear: if compression hides the wrong detail or
changes the shape of output too much, the model can get confused. Then token
savings become negative value because the model makes bad decisions, reruns
commands, asks for raw output, or patches the wrong thing.

The winning product is not "truncate harder". The winning product is a
reversible context codec:

1. Save full stdout/stderr locally.
2. Return a tiny, stable, factual digest by default.
3. Make every hidden part addressable by an exact retrieval command.
4. Escalate automatically when the model needs more detail.
5. Measure success by agent task completion, not only token reduction.

## What RTK does well today

### 1. It attacks a real source of waste

Coding agents burn a lot of context on command output:

- `cargo test`, `pytest`, `vitest`, `go test`
- `git diff`, `git status`, `git log`
- `rg`, `grep`, `find`, `tree`, `ls`
- `docker`, `kubectl`, `aws`
- package manager installs and dependency listings

Most of this output has low signal density. RTK targets exactly this layer.

### 2. It is deterministic and local

RTK uses Rust filters, TOML filters, parsers, regexes, line grouping, and caps.
That is a good default for infrastructure around agents:

- no extra LLM call for every command;
- low latency;
- reproducible behavior;
- no remote service required;
- easier to test than natural-language summarization.

### 3. It preserves normal command execution

The public architecture emphasizes exit-code propagation and fallback behavior.
This is critical. A command-output compressor must not become a shell that lies.
If `cargo test` exits with `101`, the model must see failure clearly.

### 4. It has useful command-specific semantics

RTK is not only `head -n 100`. It has specialized behavior for ecosystems:

- test filters keep failing tests and summaries;
- build filters keep errors and warnings;
- Git filters summarize status, diffs, logs, and PR data;
- file-search filters group matches by file;
- cloud/container filters can summarize inventory and health.

This is the right direction because every tool has a different "important"
shape.

### 5. It already has recovery primitives

RTK has tee output recovery: raw output can be saved to a local file, and the
compact output can include a hint to retrieve the full output. This is the seed
of the right product.

### 6. It has adoption mechanics

Hooks/plugins matter. If the agent has to remember to type `rtk`, adoption will
be bad. RTK rewrites commands through hooks for multiple agents, which makes the
optimization mostly automatic.

### 7. It has analytics

`rtk gain` and the SQLite tracking layer are useful for showing value and
finding gaps. A compression product needs local feedback loops:

- which commands are actually used;
- which filters save tokens;
- which commands bypass RTK;
- where users disable RTK.

## Product and technical problems

### 1. Compression can destroy model-relevant context

The model does not only need "the error". It often needs surrounding context:

- the exact assertion;
- the expected and actual values;
- the first stack frame inside the project;
- the tool's summary line;
- the file path and line number;
- the command arguments;
- stderr vs stdout;
- whether the output was complete or partial.

If RTK drops any of those, the model may form a wrong hypothesis.

Bad compressed output:

```text
cargo test: 2 failures
```

Good compressed output:

```text
cmd: cargo test
exit: 101
raw: run:8f31
tests: failed 2 / passed 182

FAIL tests/oauth.rs:88 refresh_token_times_out
  error: timed out after 30s
  first project frame: src/oauth/refresh.rs:143

FAIL tests/auth.rs:42 rejects_invalid_account
  assertion: expected 401, got 200

hidden: 2 stack traces, 214 log lines
expand: rtk show run:8f31 --failure refresh_token_times_out
```

### 2. The model can forget that the output is compressed

Transparent command rewriting is useful, but full invisibility is dangerous.
If the model believes it saw the complete `git diff` or complete test log, it
can over-trust the digest.

Every compressed response should include a small machine-readable header:

```text
rtk: compressed | cmd="cargo test" | exit=101 | raw=run:8f31 | hidden=214 lines
```

This costs a few tokens and prevents a lot of confusion.

### 3. Recovery is not first-class enough

A raw tee file is better than nothing, but a model should not have to inspect a
giant log manually. Recovery should be addressable:

```text
rtk show last --raw
rtk show last --stderr
rtk show last --errors
rtk show last --failure refresh_token_times_out
rtk show last --file tests/oauth.rs
rtk show last --span 120:180
rtk show last --grep "timeout"
```

The product should make the hidden detail cheap to retrieve precisely.

### 4. Static caps are too blunt

Fixed caps like "show 10 warnings" or "show 20 errors" are simple, but not
always correct. Sometimes 1 root-cause error is enough. Sometimes the model
needs the first 3 failures and the final summary. Sometimes a security/audit
command should show more.

The system needs adaptive budgets:

```text
budget=tiny     status + top root cause
budget=normal   root causes + nearby context
budget=debug    expanded blocks + selected stack frames
budget=raw      no compression
```

### 5. Shell hooks do not cover every expensive context path

In Claude Code and similar tools, built-in tools like `Read`, `Grep`, and `Glob`
can bypass shell hooks. That means RTK may save a lot on shell commands but miss
large file reads and native search outputs.

This is a product gap, not just an implementation detail. The best compression
layer should cover:

- shell command output;
- native file reads;
- native search results;
- tool responses;
- long agent history;
- repeated command outputs.

### 6. Token savings metrics are approximate

The public docs describe token estimation as roughly `chars / 4`. That is fine
for a dashboard, but not enough for a serious optimization product.

Better metrics:

- tokenizer-specific estimates for OpenAI, Anthropic, Gemini;
- saved input tokens vs saved output tokens;
- cache-read/cache-write effects where available;
- savings from avoided reruns;
- savings lost from recovery calls.

The real KPI is not "compressed 90%". The real KPI is:

```text
task success with compressed context ~= task success with raw context
while using far fewer tokens
```

### 7. Summaries can be too human and not parseable enough

Models do better with stable, factual, parseable output than with prose.

Prefer:

```text
FAIL file=tests/auth.rs line=42 test=rejects_invalid_account expected=401 actual=200
```

Avoid:

```text
Looks like an auth issue in the tests.
```

Inference can be offered, but it must be labeled as inference and kept separate
from raw facts.

### 8. Over-rewriting can change workflows

Any rewrite layer can cause edge cases:

- shell constructs that are hard to attest;
- redirects;
- pipes;
- commands that intentionally rely on full output shape;
- scripts that parse stdout;
- destructive commands where the full plan matters.

The product should default to compression for human/model display, not for data
pipes. If stdout is being consumed by another command, be conservative.

### 9. Security and privacy need stronger defaults

RTK sees command strings and raw output. That can include:

- bearer tokens;
- API keys;
- auth headers;
- secrets manager output;
- `.env` values;
- deployment logs;
- database rows.

Needed controls:

- redact secrets before tracking command strings;
- redact secrets in raw stores by policy or encrypt raw stores locally;
- short retention by default;
- per-project opt-out;
- `rtk raw` permission gate for sensitive runs;
- no telemetry that contains arguments, paths, raw output, or source snippets.

### 10. Unknown formats are dangerous

If RTK cannot parse a tool confidently, aggressive compression is risky.

Safe fallback policy:

```text
known parser, high confidence: semantic digest
known parser, low confidence: short raw head/tail + raw handle
unknown command: passthrough or minimal truncation only
security/deploy/audit command: conservative mode
```

## Product principles for a better version

### Principle 1: Do not hide the contract

Every response should say:

- command;
- exit code;
- whether output is complete;
- how much was hidden;
- raw run id;
- exact retrieval commands.

### Principle 2: Facts before interpretation

The compressed output should be mostly structured facts:

- file paths;
- line numbers;
- test names;
- status codes;
- assertion values;
- resource names;
- counts;
- exact tool error lines.

If the product includes "likely cause", put it in a separate `inference` field.

### Principle 3: Reversibility over prettiness

The model should never be trapped by the compressed view. Every hidden part needs
an address.

### Principle 4: Compression must be stateful

Repeated outputs should not be printed again. RTK should know:

- same command;
- same exit code;
- same digest hash;
- same failure signature;
- changed since previous run or not.

Example:

```text
rtk: unchanged from run:8f31
exit: 101
same failures: refresh_token_times_out, rejects_invalid_account
```

### Principle 5: Optimize for next action

The output should answer: "What should the agent do next?"

For a failing test, that means failing test name, first assertion, and source
location. For `git status`, that means changed files and staging state. For
`kubectl`, that means unhealthy resources first.

## Concrete feature ideas

### 1. Run store with stable IDs

Store every captured run as:

```text
run_id
timestamp
cwd
command
exit_code
stdout_path
stderr_path
raw_hash
digest_hash
parser_name
parser_confidence
```

Expose:

```text
rtk runs
rtk show last
rtk show run:8f31 --raw
rtk show run:8f31 --stderr
rtk show run:8f31 --json
rtk show run:8f31 --grep "timeout"
rtk show run:8f31 --span 120:180
```

### 2. Addressable semantic blocks

Parsers should emit blocks with IDs:

```text
block:test_failure:refresh_token_times_out
block:error:E0308:src/lib.rs:42
block:diff:src/oauth.rs:exchange_token
block:k8s_pod:api-7f8d:CrashLoopBackOff
```

Then the model can retrieve exactly:

```text
rtk show run:8f31 --block test_failure:refresh_token_times_out
```

### 3. Multi-level output budgets

Default output should be tiny. The model can expand on demand.

Suggested levels:

```text
L0: command, exit, counts, raw id
L1: top root causes
L2: root causes + compact context
L3: expanded failures/errors
raw: full stdout/stderr
```

Possible CLI:

```text
rtk cargo test --budget tiny
rtk cargo test --budget normal
rtk cargo test --budget debug
rtk show last --budget debug
```

### 4. Agent-facing structured mode

Human-friendly output and model-friendly output are different.

Add:

```text
RTK_AGENT_FORMAT=jsonl
RTK_AGENT_FORMAT=compact
RTK_AGENT_FORMAT=markdown
```

For agents, prefer JSONL-ish facts:

```json
{"type":"rtk_header","cmd":"cargo test","exit":101,"raw":"run:8f31","complete":false}
{"type":"test_failure","file":"tests/auth.rs","line":42,"test":"rejects_invalid_account","expected":"401","actual":"200"}
{"type":"hidden","stack_traces":2,"log_lines":214,"expand":"rtk show run:8f31 --failure rejects_invalid_account"}
```

### 5. Semantic file reader

`rtk read` should be a code-aware reader, not just a truncator.

Features:

```text
rtk read src/oauth.rs --outline
rtk read src/oauth.rs --symbol refresh_token
rtk read src/oauth.rs --imports
rtk read src/oauth.rs --callers refresh_token
rtk read src/oauth.rs --around 143
```

Implementation direction:

- tree-sitter for common languages;
- fallback to regex outline;
- always preserve line numbers;
- expose symbol-level retrieval.

### 6. Better `git diff` compression

Instead of dumping raw hunks, output:

```text
diff: 4 files, +120 -48

src/oauth.rs
  fn refresh_token changed +32 -8
  risk: auth flow, token expiry
  expand: rtk show last --diff src/oauth.rs

tests/oauth.rs
  added 2 tests
```

Keep exact hunks retrievable:

```text
rtk show last --diff src/oauth.rs --function refresh_token
```

### 7. Repetition and dedup engine

Detect repeated command outputs:

- same command and same raw hash;
- same failure signature;
- same set of failing tests;
- same compiler diagnostics.

Return only delta:

```text
unchanged failure signature from run:8f31
new since previous: src/oauth.rs changed
```

### 8. Confidence scoring

Every parser should produce a confidence score:

```text
parser=cargo_test confidence=0.97
```

If confidence is low, output more raw context and mark it:

```text
rtk: low parser confidence, showing conservative head/tail
```

### 9. Safety router

Introduce command classes:

```text
safe_to_compress: tests, builds, search, status
needs_conservative_mode: deploys, migrations, terraform plans, security scanners
passthrough_by_default: unknown commands, machine-readable pipes
```

Special handling:

- `terraform plan`: never hide creates/destroys/replaces;
- migrations: never hide migration names or destructive statements;
- security scans: never hide severity, CVE, package, fixed version;
- secrets commands: redact by default.

### 10. Better coverage for native tools

For agents where native tools bypass shell hooks, add integrations beyond Bash:

- MCP server for compressed file read/search;
- plugin adapters for agent-native tool outputs;
- instructions that route large reads through `rtk read`;
- optional wrapper tools: `rtk_read`, `rtk_grep`, `rtk_glob`.

This is probably the biggest product-level unlock after reversible run storage.

### 11. "No confusion" test suite

Add evaluation fixtures where an agent must solve a task from compressed output.

Test categories:

- failing Rust test;
- Python traceback;
- TypeScript compile error;
- large grep result;
- git conflict;
- Kubernetes CrashLoop;
- Terraform plan;
- security scanner output.

For each fixture, compare:

```text
raw output task success
compressed output task success
tokens used
number of recovery calls
wrong hypotheses produced
```

Release gates should include:

```text
compressed_task_success >= raw_task_success - small_tolerance
average_token_savings >= target
wrong_hypothesis_rate does not increase
```

### 12. Real tokenizer accounting

Keep `chars / 4` as a fallback, but add:

- OpenAI tokenizer mode;
- Anthropic estimate mode;
- Gemini estimate mode;
- model-specific dashboard;
- net savings after recovery calls.

### 13. Secret-aware local storage

Run store should support:

- raw output disabled;
- raw output encrypted;
- raw output redacted;
- per-command sensitivity labels;
- automatic deletion after N days or N runs;
- `rtk purge`.

### 14. Project-local policy

Per repo:

```toml
[compression]
default_budget = "normal"
security_mode = "conservative"

[commands]
"terraform plan" = "conservative"
"cargo test" = "tiny"
"pytest" = "normal"

[redaction]
enabled = true
```

This lets teams tune compression without retraining every agent prompt.

## Suggested product shape

### Name and positioning

RTK can remain the CLI brand, but the product concept should be:

```text
Context I/O layer for coding agents
```

Not just "token killer". The stronger promise:

```text
Give the model the minimum actionable terminal context without losing the raw truth.
```

### Default digest format

Recommended default:

```text
rtk: compressed cmd="..." exit=N raw=run:ID hidden="X lines, Y bytes"
status: ...
facts:
  ...
next:
  expand: ...
```

The output must be boring and stable. Do not optimize for pretty terminal UI in
agent mode.

### Retrieval UX

The model should learn one simple pattern:

```text
rtk show last ...
```

Everything hidden must be retrievable through this path.

### Modes

```text
human mode: pretty compact output
agent mode: stable parseable output with run ids
ci mode: conservative, exact exit/status, no hidden destructive details
debug mode: expanded context
```

## Roadmap

### Phase 1: Make current compression safer

- Add universal RTK header with command, exit code, raw id, hidden count.
- Make tee/run-store IDs stable and retrievable through `rtk show`.
- Add `rtk show last --raw`, `--stderr`, `--errors`, `--grep`, `--span`.
- Add parser confidence and conservative fallback.
- Add redaction for command strings and raw stores.

### Phase 2: Make compression more useful

- Add semantic block IDs for test failures, compiler errors, grep groups, diffs.
- Add `--budget tiny|normal|debug|raw`.
- Add repeated-output detection and digest caching.
- Add structured agent output mode.
- Add per-project compression policy.

### Phase 3: Cover more agent context

- Add native read/search/glob compression tools.
- Add MCP/plugin integrations for agents whose native tools bypass shell hooks.
- Add tree-sitter outlines and symbol retrieval.
- Add task-success evaluation harness.

### Phase 4: Optimize economics honestly

- Add real tokenizer integrations.
- Track net savings after recovery calls.
- Track rerun avoidance.
- Track task-success parity against raw output.

## Non-negotiable correctness rules

RTK-like compression should obey these rules:

1. Never report success when exit code is non-zero.
2. Never hide that output is partial.
3. Never hide the only known error line.
4. Never hide exact file path and line number when available.
5. Never hide destructive resource changes in deploy/infra plans.
6. Never make a natural-language guess look like a fact.
7. Always provide a recovery command for hidden output.
8. Always preserve stderr/stdout distinction in raw retrieval.
9. Always fall back conservatively when parser confidence is low.
10. Always make compression disable/expandable per command.

## Final recommendation

The best version of RTK is not an aggressive truncator. It is a reversible,
stateful, model-aware terminal codec.

The product should optimize for this loop:

```text
run command
store raw truth
emit tiny factual digest
let model retrieve exact missing slice
dedup repeated outputs
measure whether the model still solves the task
```

If done this way, token consumption can drop much lower than today's simple
filtering without making the model worse. The model gets less noise, but it
never loses the ability to recover the important detail.
