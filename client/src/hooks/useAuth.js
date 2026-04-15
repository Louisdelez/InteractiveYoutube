import { useEffect, useState, useCallback } from 'react';
import { api } from '../services/api';

export function useAuth() {
  const [user, setUser] = useState(null);
  const [isLoading, setIsLoading] = useState(true);

  useEffect(() => {
    api.get('/api/auth/me')
      .then((data) => {
        setUser(data.user);
        setIsLoading(false);
      })
      .catch(() => {
        setUser(null);
        setIsLoading(false);
      });
  }, []);

  const login = useCallback(async (email, password) => {
    const data = await api.post('/api/auth/login', { email, password });
    setUser(data.user);
    // Reload page to reconnect socket with new cookie
    window.location.reload();
  }, []);

  const register = useCallback(async (username, email, password) => {
    const data = await api.post('/api/auth/register', { username, email, password });
    setUser(data.user);
    window.location.reload();
  }, []);

  const logout = useCallback(async () => {
    await api.post('/api/auth/logout');
    setUser(null);
    window.location.reload();
  }, []);

  return { user, isLoading, login, register, logout };
}
