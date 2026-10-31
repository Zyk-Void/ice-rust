import { Routes } from '@angular/router';
import { ComparePage } from './pages/compare';
import { LivePage } from './pages/live';
import { OverviewPage } from './pages/overview';
import { RunDetailPage } from './pages/run-detail';
import { RunsPage } from './pages/runs';

// Eager routes: the server embeds exactly one script, so nothing lazy-loads.
export const routes: Routes = [
  { path: '', component: OverviewPage, title: 'Overview · icers observe' },
  { path: 'live', component: LivePage, title: 'Live · icers observe' },
  { path: 'runs', component: RunsPage, title: 'Runs · icers observe' },
  { path: 'runs/:runId', component: RunDetailPage, title: 'Run · icers observe' },
  { path: 'compare', component: ComparePage, title: 'Compare · icers observe' },
  { path: '**', redirectTo: '' },
];
