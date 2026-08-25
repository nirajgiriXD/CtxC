# CtxC (Context Compiler)

> Cross-platform context optimization for AI agents.

CtxC is an open-source context optimization engine designed to help AI coding agents, LLM applications, and developers reduce unnecessary token usage while preserving important information.

Instead of blindly sending large amounts of context to AI models, CtxC intelligently analyzes, transforms, and prepares high-value context optimized for reasoning.

Think of CtxC as a **compiler for AI context**:

```text
Raw Context
     |
     | Analyze
     | Filter
     | Retrieve
     | Compress
     | Optimize
     |
     v
AI-Ready Context
```

**Documentation**

- **[USAGE.md](USAGE.md)** — install, configure, and operate CtxC
- **[ARCHITECTURE.md](ARCHITECTURE.md)** — how CtxC works internally

---

## Why CtxC?

Modern AI agents are becoming increasingly capable, but context management remains one of the biggest challenges.

Large context windows introduce problems:

- Increased API costs
- Slower inference
- More irrelevant information
- Reduced reasoning quality
- Repeated context consumption
- Poor scalability for long-running agents

CtxC solves this by ensuring AI models receive the **right context, at the right time, in the right format**.

---

## Features

### Context Optimization

Reduce unnecessary tokens while maintaining meaning and important details.

- Intelligent context compression
- Duplicate information removal
- Redundant output filtering
- Semantic summarization
- Information prioritization

---

### Intelligent Context Selection

Provide AI agents with only the information relevant to the current task.

- Relevance scoring
- Semantic search
- Context ranking
- Dependency-aware retrieval
- Important information preservation

---

### Tool Output Optimization

AI agents frequently consume large amounts of unnecessary command output.

CtxC helps optimize:

- Terminal output
- Logs
- API responses
- Documentation
- Code analysis results
- Tool execution results

---

### Context Memory

Maintain useful long-term information without repeatedly sending everything.

Supports concepts such as:

- Project knowledge
- Previous decisions
- Agent memory
- Persistent context
- Knowledge relationships

---

### Cross Platform

Designed to work consistently across:

- macOS
- Linux
- Windows

---

## How It Works

CtxC acts as an intelligent middleware layer between your application and AI models.

```text
          Developer
              |
              |
              v

    AI Application / Agent

              |
              |
              v

      +---------------+
      |     CtxC      |
      |               |
      | Context       |
      | Optimization  |
      | Engine        |
      +---------------+

              |
              |
              v

         LLM Model
```

---

## Core Pipeline

```text
Input Context
      |
      v
Context Analysis
      |
      v
Relevance Detection
      |
      v
Deduplication
      |
      v
Compression
      |
      v
Context Formatting
      |
      v
Optimized Output
```

---

## Use Cases

### AI Coding Agents

Improve coding assistants by reducing unnecessary repository context.

Example:

Before:

```text
50,000 tokens:

Entire repository
Unrelated files
Duplicate documentation
Old logs
```

After:

```text
5,000 tokens:

Relevant files
Required dependencies
Project conventions
Previous decisions
```

---

### LLM Applications

Optimize prompts before sending them to models.

Useful for:

- Chat applications
- AI assistants
- Customer support bots
- Knowledge assistants
- Enterprise AI systems

---

### Developer Tools

Integrate CtxC into:

- IDE extensions
- CLI tools
- Agent frameworks
- Automation systems
- Internal AI workflows

---

## Architecture

CtxC is designed as a modular context processing system.

```text
             Context Sources

    Files     Logs     APIs     Memory
      |        |        |        |
      +--------+--------+--------+

                   |
                   v

          Context Ingestion

                   |
                   v

          Context Processing

    +----------------------------+
    |                            |
    |  Semantic Analysis         |
    |  Compression               |
    |  Ranking                   |
    |  Retrieval                 |
    |  Memory                    |
    |                            |
    +----------------------------+

                   |
                   v

          Optimized Context

                   |
                   v

                LLM
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for the full design.

---

## Design Principles

### Preserve Meaning Over Tokens

The goal is not simply reducing tokens.

A smaller context with missing information is worse than a larger context containing useful information.

CtxC optimizes for:

> Maximum Intelligence per Token

---

### Local First

Where possible, processing should happen locally.

Benefits:

- Privacy
- Lower cost
- Faster execution
- Offline capabilities

---

### Model Agnostic

CtxC should work with:

- OpenAI models
- Anthropic models
- Google models
- Local models
- Future AI models

The context layer should not depend on a specific provider.

---

### Developer Friendly

Designed for:

- CLI usage
- Automation
- Plugins
- APIs
- Agent integrations

---

## Quick Start

CtxC is a single native binary with no runtime dependency. Build it from source:

```bash
cargo build --release
```

Then shrink noisy output, index a project, and pull out only what a task needs:

```bash
# Optimize what a command printed
ctxc optimize -- cargo test

# Index a project, then assemble task-scoped context
ctxc project index .
ctxc find "why does the session expire early" --compile --budget 6000
```

Full instructions — prerequisites, installation, configuration, every command, workflows, troubleshooting — are in **[USAGE.md](USAGE.md)**.

---

## Integrations

CtxC ships adapters that tell coding agents it exists, and an MCP server that hands them its tools directly:

- Claude Code
- Codex, OpenCode, Aider, and other `AGENTS.md` readers
- GitHub Copilot
- Gemini CLI
- Cursor
- Cline

See [USAGE.md](USAGE.md#ctxc-integrations) for how to install them.

---

## Contributing

Contributions are welcome!

Areas where help is needed:

- Context optimization algorithms
- Language parsers
- AI integrations
- Performance improvements
- Cross-platform development
- Documentation

---

## Philosophy

AI models are becoming more powerful, but intelligence is still limited by the quality of the context they receive.

CtxC exists to solve one fundamental problem:

> Give AI agents less information, but make that information better.

---

## License

MIT License

---

## Acknowledgements

Inspired by the growing ecosystem of AI context optimization tools and research around:

- Retrieval-Augmented Generation (RAG)
- Semantic search
- Prompt optimization
- AI memory systems
- Knowledge graphs
- Agent architectures

CtxC combines these ideas into a unified, cross-platform context optimization layer.
