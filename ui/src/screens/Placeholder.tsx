// New for agent-router: what a screen shows until the Northstar item that builds it lands.
import type { ReactNode } from 'react';
import { Tile } from '@carbon/react';
import { EmptyState } from '~/components/EmptyState';
import styles from './Placeholder.module.scss';

/// A screen that is not built yet says so, and says where the work is tracked: an empty page
/// would read as a router with nothing in it, which is a different fact.
export function Placeholder({ item, what, children }: { item: string; what: string; children?: ReactNode }) {
  return (
    <Tile className={styles.tile}>
      <EmptyState title={`${what} is ${item}`}>
        {children} It is tracked in Northstar as {item}, in the agent-router project. Until it lands, the router's
        own pages answer at / and /live as they always have.
      </EmptyState>
    </Tile>
  );
}
