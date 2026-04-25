import { Routes } from '@angular/router';

export const routes: Routes = [
	{
		path: '',
		pathMatch: 'full',
		loadComponent: () => import('./features/home/home'),
		title: 'Glave — Humane, affordable intelligence',
	},
	{
		path: 'api',
		loadComponent: () => import('./features/api/api'),
		title: 'API · Glave',
	},
	{
		path: 'docs',
		loadComponent: () => import('./features/docs/docs'),
		title: 'Docs · Glave',
	},
	{
		path: 'chat',
		loadComponent: () => import('./features/chat/chat'),
		title: 'Chat · Glave',
	},
	{
		path: 'blog',
		loadComponent: () => import('./features/blog/blog'),
		title: 'Blog · Glave',
	},
	{
		path: 'legal',
		loadComponent: () => import('./features/legal/legal'),
		title: 'Legal · Glave',
	},
	{
		path: 'foundations',
		loadComponent: () => import('./features/foundations/foundations'),
		title: 'Foundations · Glave',
	},
];
