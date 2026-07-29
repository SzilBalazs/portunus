import TagEditor from "./TagEditor";

interface Props {
  extensions: string[];
  onChange: (e: string[]) => void;
  placeholder?: string;
}

/**
 * Tag-input for file extensions: `TagEditor` plus the leading-dot strip, so
 * ".pdf" and "pdf" both land as "pdf". Shared by the global content list and
 * each per-directory override.
 */
export default function ExtensionEditor({ extensions, onChange, placeholder = "add ext…" }: Props) {
  return (
    <TagEditor
      values={extensions}
      onChange={onChange}
      placeholder={placeholder}
      normalize={raw => raw.replace(/^\./, "")}
    />
  );
}
