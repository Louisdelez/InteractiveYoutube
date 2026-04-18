import { useState, useEffect, useCallback, useRef, useMemo } from 'react';
import { ChevronRight, History, Star, Tv } from 'lucide-react';
import { api } from '../services/api';
import './ChannelSidebar.css';

// Kept as a fallback for the first paint before `/api/tv/channels` resolves
// (and for offline dev). The server is the source of truth — anything added
// in `server/config.js` appears here automatically after a mount-time fetch.
const CHANNELS_FALLBACK = [
  { id: 'amixem', name: 'Amixem', avatar: 'https://yt3.googleusercontent.com/mkxR4YNTUBJAjuq020488wM8yHSCZ4Kwn0etJyYyGTL86LnEiIzu5uhw8EwmPpRxavKYXyQ4Hmk=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'squeezie', name: 'Squeezie', avatar: 'https://yt3.googleusercontent.com/ytc/AIdro_mPZvx-xk6pbAYdC7G8jUZzgCNDDTg1ZfF0_Lwd8UpJT4M=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'mcfly-carlito', name: 'Mcfly et Carlito', avatar: 'https://yt3.googleusercontent.com/ytc/AIdro_kSPG3h89eFoHhkLFYl_VQ6OkFpLCfpZUSuIWkRJt0sI-E=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'pierre-chabrier', name: 'Pierre Chabrier', avatar: '/avatars/pierre-chabrier.jpg' },
  { id: 'vilebrequin', name: 'Vilebrequin', avatar: 'https://yt3.googleusercontent.com/ytc/AIdro_mEdBuj1fCqUVLlvdLxIQtLZvUBN3cD-fB7LrYY9Gug65M=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'pierre-croce', name: 'Pierre Croce', avatar: 'https://yt3.googleusercontent.com/nITpEppzrNhVmiOCzBsmwQdjzaJ-qJnz4KKwqhbfXgTxdAkKP8ITAz3dlmdIOLPnm4nyxBdbOA=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'sylvain-lyve', name: 'Sylvain Lyve', avatar: 'https://yt3.googleusercontent.com/VIUzUgC_byAkR-ZVQkcTNeu1bV2DL1r4edu993uQwKMwh14vrffbnZJUKRTM-G6-JVac1d8Vd2E=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'gmk', name: 'GMK', avatar: 'https://yt3.googleusercontent.com/dUPQNmo-biSznsRa11lPuU4LMJIMCfGYspvm0eDwxh0poHr7-0BoLSc0Sx6bvW2LTUk5m1nMWg=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'thierry-vigneau', name: 'Thierry Vigneau Boiserie', avatar: 'https://yt3.googleusercontent.com/oKx9FZWczFvYXqqRpEEBvJ1K-oCyN1n1hzq-_hGlq5Kqtq4d09MQf1_zG_DqZ4vjK1WYqdRU=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'inoxtag', name: 'Inoxtag', avatar: 'https://yt3.googleusercontent.com/ytc/AIdro_nBv_ScBglsYmGLCeX8gG5E7_rC-p9M0I4hQAcEMaHjJa4=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'michou', name: 'Michou', avatar: 'https://yt3.googleusercontent.com/AbT6_C0E4bzscwpKqfdeMg6wTCuo_5pP9lkeqcLBFtqbgJsf8GaRGBAUnf7ZuNwEuiTHA7fI=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'anyme', name: 'Anyme', avatar: 'https://yt3.ggpht.com/dDEFIyDIjJwDz4rDzR5uPCkX0vxqZV3BkIqLYOo0toZiwqxRPjchDMt6ue4jQKF4tkqzQ9nV4Q=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'diablox9', name: 'DiabloX9', avatar: 'https://yt3.ggpht.com/uMCOWq8avyw0TmjgFXjbcKhIucrIL_Nrs9CfqT05DjMXiXtHHjWXe9Rzwoqokyp4kmCnVEV9Qg=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'frigiel', name: 'Frigiel', avatar: 'https://yt3.ggpht.com/ytc/AIdro_kgShYZ1YzaXOBkKZVcNEIFsG0gW8f7lRBSa6Iiztcm_OE=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'arte', name: 'Arte', avatar: 'https://yt3.ggpht.com/JvG9t1fNgirmoEWieadI9gz5wm0889z8ULzCF959u1FrDykh8LJUO9BjEGUg1k_xDOZWFNb3uQ=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'cest-pas-sorcier', name: "C'est pas sorcier", avatar: 'https://yt3.ggpht.com/ytc/AIdro_mM6iBpwKwzyyA6sGUzXdRgJbsEY_9RzW5m8GCkbqwgoHM=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'jamy-epicurieux', name: 'Jamy Epicurieux', avatar: 'https://yt3.ggpht.com/PmI8M3fQcLUOPc9oU6PO86lVIdlqwQPQ5K4T_s5YWOei67zPRJ1Knq6pzKawZNWh2Ls7H4w2Dg=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'le-monde-de-jamy', name: 'Le Monde de Jamy', avatar: 'https://yt3.ggpht.com/xnS1GlkSxjq4gSCszvOZs2JXukVrLQ_PvJS1D8qD-kGlQWAmRd2FclWf9Os0lBx6U8JsMqWoVg=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'matt-stefani', name: 'Matthieu Stefani', avatar: 'https://yt3.ggpht.com/D9-xM4azak3iUUib_iSu0TF40VzHKOWIXX_nZD7KaSVd6bDwNMZo4buthDCAr35n10ldm4lD=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'le-grand-jd', name: 'Le Grand JD', avatar: 'https://yt3.ggpht.com/ytc/AIdro_l4Pra23FO6-vPtWRXDhtSjnuSQnLF4SGsipS0Lh1MndPk=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'etienne-moustache', name: 'Etienne Moustache', avatar: 'https://yt3.ggpht.com/MQbyH0jbmKUyIHKJH4RwADwfX-EE28ZLq0eWtCOEEvdBabEv79gIZOO7r6ecG4DA86QVed21zQ=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'ivan-casta', name: 'Ivan Casta', avatar: 'https://yt3.ggpht.com/4E00nf58e3F2Jn3IqIGA7iPBkBjqRmPLKQ-zYEEQgHu8B5TJUP5i5URYomT_1DaCwPf_vruBlg=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'swan-neo', name: 'Swan et Néo', avatar: 'https://yt3.ggpht.com/ytc/AIdro_lb76hFbhH9PZ0XW0euU91CkBnzsWNNvah-eNa4KbtaiwY=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'fuze-iii', name: 'Fuze III', avatar: 'https://yt3.ggpht.com/3aC7WFMmbNW9pCriWmkLfTxx8hl_YrMQKNcmfaizLRD6v_Nt3Qmn6jAnl7XxFYfKGJzfj0EUQlE=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'joyca', name: 'Joyca', avatar: 'https://yt3.ggpht.com/F5R-8dCR4OsDu1Rs_2RE20e6LUNFDJW6VemSqToit8XvdfoSj1DXJXb0Dc4aT_YEv-5TsFCF=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'legend', name: 'Legend', avatar: 'https://yt3.ggpht.com/xS5_gZPfxkt8cXOIhWBNxKKAxDJst3Pe7TwaF-lhfbfOIB7Ctyt2R5YE6tNIfiZJpGsdyRG-=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'eloy-spinnler', name: 'Eloy Spinnler', avatar: 'https://yt3.ggpht.com/xBchl3UURdXsU88En-MX7ZvEIfWCRvJBxRtYQtaV8kMt6ET7loexyxiRGqUwVKGxivtqMBBr-w=s160-c-k-c0x00ffffff-no-rj' },
  { id: '52-minutes', name: '52 Minutes RTS', avatar: 'https://yt3.ggpht.com/CFOI3N-yzPTdB_e2CBurOPM8fUDE8hDZXimp9_qqBYePVxK443EXv0RW0QX0-zLrRyXWX0T69A=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'leo-duff', name: 'Léo Duff', avatar: 'https://yt3.ggpht.com/ajO6plfg8hOANN2taUnHF09pfikv-SMS7uLiyH6UaULUHtm_bfGZ8qPuYTpIDJolo1sZ-fpqtBs=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'romain-lanery', name: 'Romain Lanéry', avatar: 'https://yt3.ggpht.com/v2D0WfvIE1yLmfHoUZdn2dwgPiDEqAbeK5ZBXdo-ZfbZ_8db-GO9qp49wp4KtadZdZvpCf8o=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'ego', name: 'EGO', avatar: 'https://yt3.ggpht.com/7wKgNZReKXMVuRP11m-o1-6yV5YU31l_BmozxJgG457ngwWSN9a02iE4YaMyKhHsDl6ViZ5uCg=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'wartek', name: 'Wartek', avatar: 'https://yt3.ggpht.com/_ie2YptqbJLSCWuuwlo5OP5PJhzfK8VpZmFqCohUQ29qcoZQqinHwi-B9egop--weNX2LCBnYaI=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'digidix', name: 'Digidix', avatar: 'https://yt3.ggpht.com/ytc/AIdro_n82KcQlKe4En-nmP0HmQaMfoXwvPgLYTcVOHAMfVhmdQ=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'redbull', name: 'Red Bull', avatar: 'https://yt3.googleusercontent.com/ytc/AIdro_k94J7kkxpmlJe3c2iDFJpHk8SAADPpf49cN7XjomXSzEE=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'ted', name: 'TED', avatar: 'https://yt3.googleusercontent.com/ytc/AIdro_koIFcCOrvh0KThLNOiazAIDu6hcs8bjkGNwe1f6A_OYm8=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'chef-etchebest', name: 'Chef Etchebest', avatar: 'https://yt3.googleusercontent.com/TETeWVwZ0hc-qP_wWMN74dFRRcZOipEhGJ4MegZgjgKoo47wv-9igEfwLlf2lVZx2LPCIAOO=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'mike-horn', name: 'Mike Horn', avatar: 'https://yt3.googleusercontent.com/ytc/AIdro_kwBn-B7PK1diHVrgMcrkufrSZJaFgXwqgxpVrCp8-z590=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'nico-mathieux', name: 'Nico Mathieux', avatar: 'https://yt3.googleusercontent.com/jQHXPSbKfrqcGVkIFNiAej1aOePweuvioSiLa7KFtRTJ1rCYX6tZfqAJyxutDJhlxfW04g4OvAM=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'cyprien', name: 'Cyprien', avatar: 'https://yt3.googleusercontent.com/ytc/AIdro_kKiE1Vpd2RZMv057AzKdHBtqkL7ksZhZ4Huwfbr9ngUyU=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'tibo-in-shape', name: 'Tibo In Shape', avatar: 'https://yt3.googleusercontent.com/MXKYsX4ryzhIwTLMwBUvx6eoWMcaF-gsHO_PLidZMEKyFj-eKyg9u0IykU8uh7ejCAS9omOlyP8=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'la-martingale', name: 'La Martingale Media', avatar: 'https://yt3.googleusercontent.com/RL-_X0wTfBcP59fw3T_rPm44DGNx_UPA72Lesbb7lOrz8k4PmsCIe8h6CJdOvTkc8uxIFvTKHg=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'zack-nani', name: 'Zack Nani', avatar: 'https://yt3.googleusercontent.com/ytc/AIdro_kDPp1JFfNZw1d-bzKRR1k5CRHqWQrm74Eoxe7akk2Xsak=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'quotidien-tmc', name: 'Quotidien TMC', avatar: 'https://yt3.googleusercontent.com/o6bCzmWzi1wi59g4SvOgfh9BswzD4VpdGe_EW-hUZPFVg_mNuEy5FJJPtemmZNe2Rp2c-g9Pyg=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'clique-tv', name: 'Clique TV', avatar: 'https://yt3.googleusercontent.com/ytc/AIdro_lsILuhE-7OWkRBxKbIZYEptZxeCRSTsbr_lOFw_fg2DMU=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'a-la-french', name: 'A la french', avatar: 'https://yt3.googleusercontent.com/yvHMSoMsGeiRDTSWj9vpiogalQrdCcE1wV96D8-5Izmvn6HxBJPcaCBDxroB_dfKB8jR-olGYg=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'youyouu', name: 'Youyouu', avatar: 'https://yt3.googleusercontent.com/0E9d2IOm7vtg1pS7DvvPlsbXXIHhwflfYvto1DHbNYWZaCGAERUIsyj5dKD4tzRXwpq86tYDHVc=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'cocadmin', name: 'Cocadmin', avatar: 'https://yt3.googleusercontent.com/YIxkspwrV9jPGhnm9owGVaFkHWWOZ4kuvYBh9b8S7pcRhuophuweM9AZuqQrkw0cDANM8pKN7w=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'noob', name: 'Noob', avatar: '/avatars/noob.jpg' },
  { id: 'popcorn', name: 'Popcorn', avatar: 'https://yt3.ggpht.com/c--lELNYgWqiCWmZsdNwHVn7vj0IecE6RM_MLEnFopPnfMuE4MO0OCPyTD12cNmEvgeKdDJVew=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'what-the-cut', name: 'What the Cut', avatar: 'https://yt3.ggpht.com/ytc/AIdro_kRdyCH6MmWSuY9WJsXaBNNK8uvDPY0ayuBe3YGr-QIDDg=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'micode', name: 'Micode', avatar: 'https://yt3.ggpht.com/FWRkKi2u2NSqVBOyenh-Q0qpqpk562aVx6SMH-Caw6QIeZmAIFcwdA3mdpNnwW-Qm-XZHXPr=s160-c-k-c0x00ffffff-no-rj' },
  { id: 'underscore', name: 'Underscore', avatar: 'https://yt3.ggpht.com/RUb9pWwhDr8-uv4WTOOvn_c6cc1K5yHa2dPrOx7nqT8K2Ez1wYnVUQO_4PCJwMxOtZGg9vvZbw=s160-c-k-c0x00ffffff-no-rj' },
].sort((a, b) => a.name.localeCompare(b.name, 'fr'));
// Note: the fetched list is sorted at fetch-time; this const is the fallback.

export default function ChannelSidebar({
  channels: channelsProp,
  currentChannel,
  onChannelChange,
  searchQuery = '',
  history = [],
  favorites = [],
  onToggleFavorite,
}) {
  const [focusIndex, setFocusIndex] = useState(-1);
  const listRef = useRef(null);
  const itemRefs = useRef([]);

  // App.jsx is the source of truth for the channel list. If it's still
  // empty (early mount, offline), fall back to the bundled list so the
  // sidebar never renders empty.
  const channels = useMemo(
    () => (channelsProp && channelsProp.length > 0 ? channelsProp : CHANNELS_FALLBACK),
    [channelsProp]
  );
  const byId = useMemo(() => {
    const m = new Map();
    for (const c of channels) m.set(c.id, c);
    return m;
  }, [channels]);

  // History items: resolve ids against the current channel list, skip
  // any stale ids. Cap at 5.
  const historyItems = useMemo(
    () =>
      history
        .map((id) => byId.get(id))
        .filter(Boolean)
        .slice(0, 5),
    [history, byId]
  );

  // Favourite items: same resolution, sorted alphabetically.
  const favoriteItems = useMemo(
    () =>
      favorites
        .map((id) => byId.get(id))
        .filter(Boolean)
        .sort((a, b) => a.name.localeCompare(b.name, 'fr')),
    [favorites, byId]
  );

  const filtered = useMemo(() => {
    if (!searchQuery.trim()) return channels;
    const q = searchQuery.toLowerCase();
    return channels.filter((ch) => ch.name.toLowerCase().includes(q));
  }, [searchQuery, channels]);

  // Flat list of all rendered items, in order: history, favoris, all.
  // Used by arrow-key navigation so it traverses every visible row.
  const allRendered = useMemo(() => {
    const rows = [];
    if (!searchQuery.trim()) {
      for (const c of historyItems) rows.push(c);
      for (const c of favoriteItems) rows.push(c);
    }
    for (const c of filtered) rows.push(c);
    return rows;
  }, [historyItems, favoriteItems, filtered, searchQuery]);

  // Scroll focused item into view
  useEffect(() => {
    if (focusIndex >= 0 && itemRefs.current[focusIndex]) {
      itemRefs.current[focusIndex].scrollIntoView({ block: 'nearest' });
    }
  }, [focusIndex]);

  // Reset focus when search changes
  useEffect(() => {
    setFocusIndex(-1);
  }, [searchQuery]);

  const handleKeyDown = useCallback((e) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setFocusIndex((prev) => Math.min(prev + 1, allRendered.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setFocusIndex((prev) => Math.max(prev - 1, 0));
    } else if (e.key === 'Enter' && focusIndex >= 0 && focusIndex < allRendered.length) {
      e.preventDefault();
      onChannelChange(allRendered[focusIndex].id);
    }
  }, [focusIndex, allRendered, onChannelChange]);

  // Render one channel row. Right-click toggles favourite. Click switches.
  let rowIndex = -1;
  const renderRow = (ch, keyPrefix) => {
    rowIndex += 1;
    const i = rowIndex;
    const isFav = favorites.includes(ch.id);
    return (
      <button
        key={`${keyPrefix}-${ch.id}`}
        ref={(el) => (itemRefs.current[i] = el)}
        className={`channel-item${ch.id === currentChannel ? ' active' : ''}${i === focusIndex ? ' focused' : ''}`}
        title={`${ch.name}${isFav ? ' ★' : ''}\nClic droit: ${isFav ? 'retirer des' : 'ajouter aux'} favoris`}
        onClick={() => onChannelChange(ch.id)}
        onContextMenu={(e) => {
          e.preventDefault();
          if (onToggleFavorite) onToggleFavorite(ch.id);
        }}
      >
        {i === focusIndex && <ChevronRight size={12} className="channel-focus-arrow" />}
        <img src={ch.avatar} alt={ch.name} className="channel-avatar" />
        {isFav && <Star size={10} className="channel-fav-badge" fill="currentColor" />}
      </button>
    );
  };

  const showSections = !searchQuery.trim();

  return (
    <div className="channel-sidebar" tabIndex={0} onKeyDown={handleKeyDown}>
      <div className="channel-list" ref={listRef}>
        {showSections && historyItems.length > 0 && (
          <>
            <div className="channel-section-header" title="Historique — clic droit sur une chaîne pour la mettre en favori">
              <History size={12} />
            </div>
            {historyItems.map((ch) => renderRow(ch, 'h'))}
            <div className="channel-section-sep" />
          </>
        )}
        {showSections && favoriteItems.length > 0 && (
          <>
            <div className="channel-section-header" title="Favoris">
              <Star size={12} />
            </div>
            {favoriteItems.map((ch) => renderRow(ch, 'f'))}
            <div className="channel-section-sep" />
          </>
        )}
        {showSections && (
          <div className="channel-section-header" title="Toutes les chaînes">
            <Tv size={12} />
          </div>
        )}
        {filtered.map((ch) => renderRow(ch, 'a'))}
      </div>
    </div>
  );
}
