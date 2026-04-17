# tsgen

A grammar-based program generator driven by tree-sitter grammars. Point it at
a `grammar.json` from any tree-sitter parser and it produces a corpus of
syntactically-structured programs in that language.

It is useful for setting up fuzzing or differential-testing campaigns and building fuzzing corpora.

## What it does

Given a tree-sitter grammar, `tsgen`:

1. Walks the grammar rules recursively from the root.
2. At each `CHOICE` node, picks one alternative
3. At each `REPEAT`/`REPEAT1`, picks a random count up to `--max-repeat`.
4. At each `PATTERN` (a terminal regex), samples from a pre-built dictionary
   of candidates — or, if you give it one, from your own identifier dict
   and/or harvested values from real source code.
5. Optionally validates each candidate program with the compiled tree-sitter
   parser and drops anything that doesn't parse.
6. Tracks per-choice coverage and keeps going until either `--count` programs
   are collected and `--coverage-target` is met, or `--max-attempts` runs out.

The output is a directory of files, one program per file.

## Quick start

Minimum inputs: a tree-sitter `grammar.json` and optionally the compiled
parser `.so` for validation.

```
cargo run --release -- \
  --grammar path/to/grammar.json \
  --parser  path/to/parser.so \
  --count 200 \
  --output-dir corpus \
  --ext .lang
```

`--parser` argument is optional, but without it the tool will generate more garbage.

## Concrete example: Cairo

As a concrete example, let's consider how to seed the corpora to bootstrap the [Cairo](https://github.com/starkware-libs/cairo/) compiler fuzzing campaign.

### 1. Build the Cairo tree-sitter grammar

Clone [tree-sitter-cairo](https://github.com/starkware-libs/tree-sitter-cairo)
and produce both `grammar.json` (tsgen input) and `parser.so` (validator):

```
git clone https://github.com/starkware-libs/tree-sitter-cairo
cd tree-sitter-cairo
tree-sitter generate
cc -O2 -fPIC -shared -I src src/parser.c -o parser.so
cd ..
```

You need the `tree-sitter` CLI installed (`cargo install tree-sitter-cli` or
`npm i -g tree-sitter-cli`).

### 2. Build an identifier dict from Cairo's corelib

Grab the Cairo compiler repo and scrape identifiers out of `corelib/` so the
generated programs use names that actually exist:

```
git clone --depth 1 https://github.com/starkware-libs/cairo
grep -rhoE '\b(fn|struct|enum|trait|impl|type|const|mod)\s+[A-Za-z_][A-Za-z0-9_]*' \
  cairo/corelib/src/ | awk '{print $2}' | sort -u > /tmp/cairo-dict.txt
```

That gives ~3.9k declared names (`Array`, `ByteArray`, `ContractAddress`,
`felt252`, `assert_eq`, `AccountContractDispatcher`, …). Extracting only
declarations keeps out doctest strings and prose from doc comments.

### 3. Generate the corpus

```
cargo run --release -- \
  --grammar tree-sitter-cairo/src/grammar.json \
  --parser  tree-sitter-cairo/parser.so \
  --count 2000 --coverage-target 0.0 \
  --max-attempts 50000 \
  --top-level-rule _declaration_statement \
  --dict /tmp/cairo-dict.txt \
  --harvest-weight 0.8 \
  --output-dir corpus-cairo --ext .cairo \
  --seed 42
```

`--top-level-rule _declaration_statement` keeps stray expression-statements
off the top level. `--harvest-weight 0.8` biases identifier slots toward the
dict. Bump `--count` / `--max-attempts` if you need a bigger corpus.

### 4. Minimize the corpus against your harness

Raw output is still noisy — plenty of programs that parse but exercise the
same compiler paths. Run `afl-cmin` against your harness to keep only the
inputs that hit unique coverage:

```
afl-cmin -i corpus-cairo -o corpus-cairo-min -- ./your_harness @@
```

See [the AFL++ guide](https://aflplus.plus/docs/fuzzing_in_depth/#b-making-the-input-corpus-unique).

### Expected result

From a 10000-program run with different options, expect on the order of **~700 unique files** after
`afl-cmin` — enough to seed a fuzzing campaign against the Cairo compiler.

## How generation actually works

### Min-depth pre-pass
Before generation, tsgen computes the minimum syntax-tree depth required to
finish expanding every rule. During generation, when the current depth
approaches `--max-depth`, the rule-expander avoids `CHOICE` alternatives and
`REPEAT` expansions that would blow past the budget. This is what keeps
generation from infinitely recursing into `expression → binary_op →
expression → ...`.

### Terminal dictionary
tsgen scans the grammar for every `PATTERN` regex, classifies it as one of
`{Identifier, DecimalNumber, HexNumber, StringLit, Whitespace, Unknown}`, and
pre-builds a candidate list using a small set of built-in defaults plus
`rand_regex` for anything weird. Identifier candidates are filtered against
the grammar's keyword set so you don't get `let = let`.

### Harvest pool and dict
For realistic output you can feed tsgen real material:

- **`--dict <file>`** (repeatable): newline-delimited identifier list. Blank
  lines and `#` comments are skipped. Loaded directly into the
  `Identifier` pool, no regex filtering, no length minimum.
- **`--harvest-dir <dir>`** (repeatable): recursively scans files and scrapes
  identifiers (≥3 chars), decimals, hex, and string literals into their
  respective pools. Use `--harvest-ext .cairo` or a glob like
  `--harvest-ext "generated_*.move"` to filter files.

Both sources merge into one pool. At every terminal-expansion site, tsgen
flips a coin weighted by `--harvest-weight` (default 0.5):

- **heads** → classify the pattern, look up that kind in the pool, return a
  random value.
- **tails** (or pool miss for that kind) → fall back to the pre-built
  candidate list.

Per-slot, independent. Higher weight = more realistic-looking output, less
regex-weird gibberish.

### Validation loop
If `--parser` is provided, every generated program is run through the
tree-sitter parser. Programs with parse errors are discarded and do **not**
count toward `--count` or `valid_coverage`. Unvalidated attempts still
contribute to *exploration* coverage.

### Coverage
Two counters are tracked:

- **exploration coverage** — which CHOICE alternatives have been *attempted*
  across all generated programs (valid or not).
- **valid coverage** — which alternatives have been reached inside programs
  that actually parsed.

The loop stops when both `programs.len() >= --count` **and** `valid_coverage
>= --coverage-target` (default 0.95). Set `--coverage-target 0.0` to make
`--count` a hard ceiling and stop the moment you have enough programs.

## CLI flags

### Core
| flag | default | notes |
|---|---|---|
| `--grammar <file>` | required | path to `grammar.json` |
| `--parser <file>` | *(none)* | compiled tree-sitter `.so`; without it, no validation |
| `--count <N>` | 100 | minimum valid programs to collect |
| `--output-dir <dir>` | `corpus` | where files get written |
| `--ext <.ext>` | `.txt` | file extension for generated files |
| `--seed <N>` | 0 | RNG seed for reproducibility |
| `--dry-run` | off | print programs to stdout, don't write files |
| `--dump-grammar` | off | dump rule/min-depth/terminal debug info and exit |

### Shape of the generated tree
| flag | default | notes |
|---|---|---|
| `--max-depth <N>` | 15 | upper bound on syntax-tree depth |
| `--max-repeat <N>` | 5 | upper bound for `REPEAT` expansions |
| `--complexity-bias <f>` | 0.0 | 0 = uniform CHOICE, 1 = strongly prefer complex alternatives when there's depth budget |
| `--top-level-rule <name>` | *(none)* | repeatable; forces the top-level expansion to pick only from these rules. e.g. skip expression-statements at file scope in C/Rust-like grammars |

### Terminal content
| flag | default | notes |
|---|---|---|
| `--dict <file>` | *(none)* | repeatable; newline-delimited identifier list |
| `--harvest-dir <dir>` | *(none)* | repeatable; scrapes ids/numbers/strings from real source |
| `--harvest-ext <filter>` | *(any)* | extension (`.cairo`) or glob (`"generated_*.move"`) |
| `--harvest-weight <f>` | 0.5 | probability of pulling from dict/harvest vs. built-ins |
| `--unicode` | off | allow non-ASCII in output (default replaces non-ASCII with `z`) |

### Stopping conditions
| flag | default | notes |
|---|---|---|
| `--coverage-target <f>` | 0.95 | valid-coverage ratio that triggers early stop once `--count` is also met |
| `--max-attempts <N>` | 10000 | hard cap on generation attempts (valid + discarded + dupes) |
| `--no-cleanup` | off | disable whitespace post-processing |

## Limitations

- **External tokens (`externals: [...]`) are opaque.** Their matching logic
  lives in a hand-written `scanner.c`; we only see the symbol name and emit
  `<MISSING:name>` at those slots. Affects Python, Ruby, Haskell. Cairo,
  Yul, most Rust-family DSLs have empty externals and are unaffected.
- **Parser-only grammar fields are ignored:** `conflicts`, `precedences`,
  `inline`, `supertypes`, `word`, `reserved`. The compiled `.so` still
  enforces them during validation.
- **Regex is JS-flavoured.** We strip lookarounds and backrefs before
  handing patterns to `rand_regex`. Unicode-property escapes and weirder
  JS-only constructs may fall through to the `"UNKNOWN"` fallback.
- **Syntactic, not semantic.** Output breaks type rules, scoping, and
  references — intentional, exercises later compiler stages.
- **`--count` is a floor, not a ceiling.** Loop keeps going until
  `coverage-target` is also met. Pass `--coverage-target 0.0` for hard stop.
