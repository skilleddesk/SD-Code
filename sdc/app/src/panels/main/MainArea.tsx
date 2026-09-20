import { TabStrip } from '../tabs';
import { DegradedBanner } from './DegradedBanner';
import { MainContent } from './MainContent';

/**
 * The main region - spec section 7.4, top to bottom:
 *
 *   #degradedBanner   only while a host is unreachable
 *   .tabstrip         the open chats
 *   #mainContent      the empty state, one pane, or two
 *
 * `Shell.tsx` re-exports this as `MainArea`; the region element (`.main`) and its column layout come
 * from src/layout/Shell.css, so this component only fills it. The banner and the strip are fixed
 * heights and the content takes the rest, which is what makes the prompt area sit at the bottom of
 * the pane rather than at the bottom of the window.
 */
export function MainArea() {
  return (
    <main className="main">
      <DegradedBanner />
      <TabStrip />
      <MainContent />
    </main>
  );
}
