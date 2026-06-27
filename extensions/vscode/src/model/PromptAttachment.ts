import type { ContentBlock } from "../shared/acpTypes";

export type PromptAttachmentKind =
  | "selection"
  | "file"
  | "diagnostics"
  | "terminal-output"
  | "changed-files"
  | "history";

export interface PromptAttachment {
  id: string;
  kind: PromptAttachmentKind;
  label: string;
  uri?: string | undefined;
  mimeType?: string | undefined;
  text?: string | undefined;
}

export interface AttachmentTransportOptions {
  embeddedContext?: boolean | undefined;
}

export function attachmentToContentBlock(
  attachment: PromptAttachment,
  options: AttachmentTransportOptions = {}
): ContentBlock {
  const canEmbedContext = options.embeddedContext !== false;
  if (attachment.text !== undefined && attachment.uri && canEmbedContext) {
    return {
      type: "resource",
      resource: {
        uri: attachment.uri,
        mimeType: attachment.mimeType ?? "text/plain",
        text: attachment.text
      }
    };
  }

  if (attachment.text !== undefined && attachment.uri) {
    return {
      type: "text",
      text: `Context from ${attachment.label}:\n\n${attachment.text}`
    };
  }

  if (attachment.uri) {
    return {
      type: "resource_link",
      uri: attachment.uri,
      name: attachment.label,
      title: attachment.label,
      mimeType: attachment.mimeType ?? "text/plain"
    };
  }

  return {
    type: "text",
    text: attachment.text ?? attachment.label
  };
}
