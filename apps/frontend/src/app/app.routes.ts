import { Routes } from '@angular/router';

export const routes: Routes = [
	{
		path: 'foundations',
		loadComponent: () => import('./features/foundations/foundations'),
		title: 'Foundations · Glave',
	},
	{
		path: '',
		pathMatch: 'full',
		redirectTo: 'foundations',
	},
];
