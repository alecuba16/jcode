import JCodeKit
import SwiftUI

/// Main conversation screen.
struct ChatView: View {
    @Environment(AppModel.self) private var model
    @State private var showSettings = false

    var body: some View {
        @Bindable var model = model
        VStack(spacing: 0) {
            header

            if let banner = model.session.errorBanner {
                ErrorBanner(message: banner) {
                    model.dismissError()
                }
                .padding(.bottom, 8)
            }

            if !model.session.notices.isEmpty {
                NoticeStack(
                    notices: model.session.notices,
                    onDismiss: { model.dismissNotice($0) }
                )
                .padding(.bottom, 8)
            }

            TranscriptView(
                entries: model.session.transcript,
                isReasoning: model.session.isReasoning
            )

            Composer(
                draft: $model.draft,
                isProcessing: model.session.isProcessing,
                isConnected: model.isConnected,
                onSend: { model.sendDraft() },
                onInterrupt: { model.interrupt() }
            )
        }
        .sheet(isPresented: $showSettings) {
            SettingsView()
        }
    }

    private var header: some View {
        HStack(spacing: 8) {
            VStack(alignment: .leading, spacing: 4) {
                Text(model.session.sessionTitle ?? model.activeServer?.serverName ?? "jcode")
                    .font(Theme.mono(16, weight: .semibold))
                    .foregroundStyle(Theme.textPrimary)
                    .lineLimit(1)
                if let modelName = model.session.modelName {
                    Text(modelName)
                        .font(Theme.mono(11))
                        .foregroundStyle(Theme.textTertiary)
                        .lineLimit(1)
                }
            }
            Spacer()
            StatusPill(phase: model.session.phase)
            Button {
                showSettings = true
            } label: {
                Image(systemName: "ellipsis.circle")
                    .font(.title3)
                    .foregroundStyle(Theme.textSecondary)
                    .frame(width: 44, height: 44)
            }
            .accessibilityLabel("Settings")
            .accessibilityHint("Sessions, model, and servers")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 8)
    }
}
