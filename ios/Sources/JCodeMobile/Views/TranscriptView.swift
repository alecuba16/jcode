import JCodeKit
import SwiftUI

/// Scrolling transcript with auto-follow.
///
/// Short threads are anchored to the bottom (chat convention) so a couple of
/// messages don't float at the top above a large dead zone; once the content
/// exceeds the viewport it scrolls normally. An empty session shows a centered
/// placeholder instead of a blank canvas.
struct TranscriptView: View {
    let entries: [TranscriptEntry]
    let isReasoning: Bool

    var body: some View {
        if entries.isEmpty && !isReasoning {
            EmptyTranscript()
        } else {
            scroller
        }
    }

    private var scroller: some View {
        ScrollViewReader { proxy in
            ScrollView {
                // A flexible top spacer pushes short content to the bottom of
                // the viewport; it collapses to zero once content overflows.
                LazyVStack(alignment: .leading, spacing: 16) {
                    Spacer(minLength: 0)
                    ForEach(entries) { entry in
                        EntryView(entry: entry)
                            .id(entry.id)
                    }
                    if isReasoning {
                        HStack(spacing: 8) {
                            ProgressView()
                                .controlSize(.small)
                                .tint(Theme.textTertiary)
                            Text("thinking")
                                .font(Theme.mono(12))
                                .foregroundStyle(Theme.textTertiary)
                        }
                        .padding(.leading, 4)
                    }
                    Color.clear.frame(height: 1).id("bottom")
                }
                .frame(minHeight: viewportMinHeight, alignment: .bottom)
                .padding(.horizontal, 16)
                .padding(.vertical, 8)
            }
            .scrollDismissesKeyboard(.interactively)
            .onChange(of: entries.last?.text) {
                withAnimation(.easeOut(duration: 0.15)) {
                    proxy.scrollTo("bottom", anchor: .bottom)
                }
            }
            .onChange(of: entries.count) {
                proxy.scrollTo("bottom", anchor: .bottom)
            }
        }
    }

    // A large min height makes the LazyVStack at least fill the viewport so
    // the bottom alignment can take effect; the ScrollView absorbs any excess.
    private var viewportMinHeight: CGFloat { 600 }
}

/// Friendly placeholder for a fresh session, centered in the canvas.
struct EmptyTranscript: View {
    var body: some View {
        VStack(spacing: 16) {
            Image(systemName: "terminal")
                .font(Theme.icon(40, weight: .light))
                .foregroundStyle(Theme.mint)
            Text("Ready when you are")
                .font(Theme.mono(16, weight: .medium))
                .foregroundStyle(Theme.textPrimary)
            Text("Send a message to start driving this session.")
                .font(.subheadline)
                .foregroundStyle(Theme.textSecondary)
                .multilineTextAlignment(.center)
        }
        .padding(32)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

/// One transcript entry: user bubble, assistant markdown, or system note.
struct EntryView: View {
    let entry: TranscriptEntry

    var body: some View {
        switch entry.role {
        case .user:
            HStack {
                Spacer(minLength: 48)
                Text(entry.text)
                    .font(.body)
                    .foregroundStyle(Theme.textPrimary)
                    .padding(12)
                    .background(Theme.mintTint)
                    .clipShape(RoundedRectangle(cornerRadius: 16))
            }
        case .assistant:
            VStack(alignment: .leading, spacing: 8) {
                if !entry.reasoning.isEmpty {
                    Text(entry.reasoning)
                        .font(Theme.mono(12))
                        .italic()
                        .foregroundStyle(Theme.textTertiary)
                        .lineLimit(4)
                }
                ForEach(entry.toolCalls) { call in
                    ToolCallCard(call: call)
                }
                if !entry.text.isEmpty {
                    MarkdownText(entry.text)
                }
            }
        case .system:
            Text(entry.text)
                .font(.footnote)
                .foregroundStyle(Theme.textTertiary)
                .frame(maxWidth: .infinity, alignment: .center)
        }
    }
}
