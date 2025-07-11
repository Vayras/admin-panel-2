import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { createBrowserRouter, RouterProvider } from 'react-router-dom';

import Login from './pages/Login.tsx';
import TableView from './pages/TableView.tsx';
import { CohortSelection } from './pages/CohortSelection.tsx';
import { ResultPage } from './pages/ResultPage.tsx';
// import StudentDetailPage from './StudentsPage.tsx';
import ProtectedRoute from './components/ProtectedRoute.tsx';
import StudentDetailPage from './pages/StudentDetailPage.tsx';


import 'virtual:uno.css';

// 🧭 Router setup
const router = createBrowserRouter([
  {
    path: '/',
    element: <Login />,
  },
  {
    path: '/select',
    element: <ProtectedRoute element={<CohortSelection />} />,
  },
  {
    path: '/admin',
    element: <ProtectedRoute element={<TableView />} />,
  },
  {
    path: '/student',
    element: <StudentDetailPage />,
  },
  {
    path: '/result',
    element: <ResultPage />,
  },
]);

// 🚀 App bootstrap
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <RouterProvider router={router} />
  </StrictMode>
);