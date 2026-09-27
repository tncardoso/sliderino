# Sliderino

Sliderino is presentation software for the agents age: a simple visual editor with complete programmatic control. People compose slides directly, and external agents work through CLI and MCP interfaces. Presentations are self-contained files that users own.

This document defines the intended product for contributors and future implementers. It describes product commitments, not features already available or a technical implementation plan.

## Compose slides with a few primitives

Sliderino provides a what-you-see-is-what-you-get (WYSIWYG) editor inspired by Figma's direct manipulation and small set of composable primitives. Authors build slides from text, images, basic shapes, and groups. They position, resize, align, and stack elements, then apply simple styling.

Sensible defaults help authors produce readable slides without navigating a large collection of specialized tools. The editor remains focused on presentations.

Layouts are fixed arrangements. Authors control element positions and dimensions. Replacing content does not automatically move neighboring elements or shrink text. A two-column slide retains its arrangement when an author changes a paragraph; the author decides whether the new content needs more space.

The preview represents the intended export appearance. When text exceeds its available space, the editor makes the overflow visible without silently changing the design.

## Build a reusable component library

Each presentation has its own component library. An author can save an element or group as a component and insert it elsewhere in the presentation. Groups containing components can become larger reusable components.

A component can be a small content unit or a full slide arrangement. Examples include a KPI box containing a number and descriptive text, and a two-column layout containing headings and content areas. Authors build these from the same primitives they use on ordinary slides.

Inserting a component creates an independent, editable copy. Changing a KPI number, description, or appearance in one copy affects neither the library component nor other copies. Editing the library component changes future insertions only.

Components therefore provide reusable starting arrangements. They do not establish live relationships between a library definition and existing slides. This copy behavior also applies to components composed from other components.

## Give external agents complete control

Sliderino supplies integrations and APIs for external agents. It does not provide or orchestrate an agent, and authors choose their own automation tools.

Both the command-line interface (CLI) and Model Context Protocol (MCP) expose the same core capabilities:

- Create, inspect, save, and modify presentations.
- Add, reorder, edit, and remove slides.
- Inspect and manipulate visual elements, groups, and styles.
- Create, inspect, edit, and reuse library components.
- Render previews and inspect layout diagnostics.
- Undo changes and export presentations.

These workflows work without opening the graphical editor. Agents can inspect document structure and rendered results to guide their changes. Text overflow is available as machine-readable diagnostic information, so an external agent can detect a problem and adjust the slide.

## Share a live editing session

When a presentation is open, changes through CLI or MCP appear immediately in the editor. A person can continue editing a presentation created by an agent, or observe an external agent modifying the current presentation.

Human and agent operations share one chronological undo history. Undo reverses the latest operation regardless of who made it. Related programmatic changes can form one undoable batch, such as repositioning several elements together.

Edits based on stale document state fail clearly instead of overwriting newer work. The technical design must define how clients detect and recover from that failure.

## Own and export the presentation

Presentations are local, self-contained files. They carry their images and supported fonts so that opening a presentation on another machine does not require finding missing assets. The supported font set must permit packaging with presentations.

Core editing, rendering, and export work without a hosted service. Choosing a desktop application or a browser-based interface is a later implementation decision.

Sliderino supports three export formats:

- **PDF:** fixed-layout slides for sharing and printing.
- **PPTX:** editable slide content, including text, shapes, and images.
- **HTML:** presentations viewable in a browser.

Export compatibility constrains the editor's visual feature set. A feature belongs in that set only if all three formats can represent it reliably. Reusable components export as their resulting slide elements; PPTX need not preserve Sliderino's component-library semantics.

External viewers may render differently, so identical output in every viewer is not a promise. Exports must preserve the intended layout shown in the preview. Text overflow produces a warning but does not block export or trigger automatic resizing.

## Keep the initial product focused

The initial product excludes Markdown authoring, automatic layout, linked component instances, cross-presentation libraries, built-in agents, and hosted collaboration.

Technology choices, storage formats, API syntax, synchronization mechanisms, and delivery schedules belong in later specifications.

## Recognize success

The vision succeeds when these workflows are possible:

- A person creates and refines a presentation visually.
- An external agent creates, inspects, revises, previews, and exports a presentation through either CLI or MCP.
- A person watches programmatic edits appear and undoes an operation or batch.
- An author saves and reuses a KPI box, a two-column layout, and a composition of components.
- Editing a component copy leaves other copies unchanged; library edits affect future insertions.
- Overflow produces visible and machine-readable warnings without automatic layout changes.
- A presentation opens on another machine with its required assets.
- The same presentation exports to PDF, editable PPTX, and HTML.
