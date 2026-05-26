import { registerProvider } from './registry';
import DictPreview from '../components/DictPreview';

registerProvider({
  kinds: ['dict', 'dict-hint'],
  Preview: DictPreview,
  handleLaunch: () => false,
});
