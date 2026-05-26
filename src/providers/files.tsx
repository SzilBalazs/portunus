import FilePreview from '../components/FilePreview';
import { registerProvider, type PreviewProps } from './registry';

function FilePreviewWrapper({ result }: PreviewProps) {
  return <FilePreview result={result} />;
}

registerProvider({
  kinds: ['file', 'folder'],
  Preview: FilePreviewWrapper,

  handleKeyDown: (e, result) => {
    if (e.ctrlKey && !e.altKey && e.key === 'c' && (result?.kind === 'file' || result?.kind === 'folder')) {
      e.preventDefault();
      const path = result.subtitle ? `${result.subtitle}/${result.title}` : result.title;
      navigator.clipboard.writeText(path);
      return true;
    }
    return false;
  },
});
