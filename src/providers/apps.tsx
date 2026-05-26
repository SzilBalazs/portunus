import AppPreview from '../components/AppPreview';
import { registerProvider, type PreviewProps } from './registry';

function AppPreviewWrapper({ result, onLaunch }: PreviewProps) {
  return <AppPreview result={result} onLaunch={onLaunch} />;
}

registerProvider({
  kinds: ['app'],
  Preview: AppPreviewWrapper,
});
