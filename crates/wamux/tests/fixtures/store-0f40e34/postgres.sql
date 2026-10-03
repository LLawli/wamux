--
-- PostgreSQL database dump
--


-- Dumped from database version 16.15 (Debian 16.15-1.pgdg13+2)
-- Dumped by pg_dump version 16.15 (Debian 16.15-1.pgdg13+2)

SET statement_timeout = 0;
SET lock_timeout = 0;
SET idle_in_transaction_session_timeout = 0;
SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;
SELECT pg_catalog.set_config('search_path', '', false);
SET check_function_bodies = false;
SET xmloption = content;
SET client_min_messages = warning;
SET row_security = off;

SET default_tablespace = '';

SET default_table_access_method = heap;

--
-- Name: _sqlx_migrations; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public._sqlx_migrations (
    version bigint NOT NULL,
    description text NOT NULL,
    installed_on timestamp with time zone DEFAULT now() NOT NULL,
    success boolean NOT NULL,
    checksum bytea NOT NULL,
    execution_time bigint NOT NULL
);


--
-- Name: accounts; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.accounts (
    uuid uuid NOT NULL,
    external_ref text,
    device_id integer NOT NULL,
    push_name text,
    created_at bigint DEFAULT (EXTRACT(epoch FROM now()))::bigint NOT NULL
);


--
-- Name: accounts_device_id_seq; Type: SEQUENCE; Schema: public; Owner: -
--

ALTER TABLE public.accounts ALTER COLUMN device_id ADD GENERATED ALWAYS AS IDENTITY (
    SEQUENCE NAME public.accounts_device_id_seq
    START WITH 1
    INCREMENT BY 1
    NO MINVALUE
    NO MAXVALUE
    CACHE 1
);


--
-- Name: app_state_keys; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.app_state_keys (
    key_id bytea NOT NULL,
    key_data bytea NOT NULL,
    device_id integer NOT NULL
);


--
-- Name: app_state_mutation_macs; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.app_state_mutation_macs (
    name text NOT NULL,
    version bigint NOT NULL,
    index_mac bytea NOT NULL,
    value_mac bytea NOT NULL,
    device_id integer NOT NULL
);


--
-- Name: app_state_versions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.app_state_versions (
    name text NOT NULL,
    state_data bytea NOT NULL,
    device_id integer NOT NULL
);


--
-- Name: base_keys; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.base_keys (
    address text NOT NULL,
    message_id text NOT NULL,
    base_key bytea NOT NULL,
    device_id integer NOT NULL,
    created_at bigint DEFAULT (EXTRACT(epoch FROM now()))::bigint NOT NULL
);


--
-- Name: blob_format; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.blob_format (
    id integer NOT NULL,
    format text NOT NULL,
    CONSTRAINT blob_format_id_check CHECK ((id = 1))
);


--
-- Name: device; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.device (
    device_id integer NOT NULL,
    data bytea NOT NULL,
    created_at bigint DEFAULT (EXTRACT(epoch FROM now()))::bigint NOT NULL
);


--
-- Name: device_registry; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.device_registry (
    user_id text NOT NULL,
    devices_json text NOT NULL,
    "timestamp" bigint NOT NULL,
    phash text,
    device_id integer NOT NULL,
    updated_at bigint DEFAULT (EXTRACT(epoch FROM now()))::bigint NOT NULL,
    raw_id integer
);


--
-- Name: identities; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.identities (
    address text NOT NULL,
    key bytea NOT NULL,
    device_id integer NOT NULL
);


--
-- Name: lid_pn_mapping; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.lid_pn_mapping (
    lid text NOT NULL,
    phone_number text NOT NULL,
    created_at bigint NOT NULL,
    learning_source text NOT NULL,
    updated_at bigint NOT NULL,
    device_id integer NOT NULL
);


--
-- Name: msg_secrets; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.msg_secrets (
    chat text NOT NULL,
    sender text NOT NULL,
    msg_id text NOT NULL,
    secret bytea NOT NULL,
    expires_at bigint DEFAULT 0 NOT NULL,
    message_ts bigint DEFAULT 0 NOT NULL,
    device_id integer NOT NULL
);


--
-- Name: prekeys; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.prekeys (
    id integer NOT NULL,
    key bytea NOT NULL,
    uploaded boolean DEFAULT false NOT NULL,
    device_id integer NOT NULL
);


--
-- Name: sender_key_devices; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.sender_key_devices (
    group_jid text NOT NULL,
    device_jid text NOT NULL,
    has_key integer DEFAULT 0 NOT NULL,
    device_id integer NOT NULL,
    updated_at bigint DEFAULT (EXTRACT(epoch FROM now()))::bigint NOT NULL
);


--
-- Name: sender_keys; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.sender_keys (
    address text NOT NULL,
    record bytea NOT NULL,
    device_id integer NOT NULL
);


--
-- Name: sent_messages; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.sent_messages (
    chat_jid text NOT NULL,
    message_id text NOT NULL,
    payload bytea NOT NULL,
    device_id integer NOT NULL,
    created_at bigint DEFAULT (EXTRACT(epoch FROM now()))::bigint NOT NULL
);


--
-- Name: sessions; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.sessions (
    address text NOT NULL,
    record bytea NOT NULL,
    device_id integer NOT NULL
);


--
-- Name: signed_prekeys; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.signed_prekeys (
    id integer NOT NULL,
    record bytea NOT NULL,
    device_id integer NOT NULL
);


--
-- Name: tc_tokens; Type: TABLE; Schema: public; Owner: -
--

CREATE TABLE public.tc_tokens (
    jid text NOT NULL,
    token bytea NOT NULL,
    token_timestamp bigint NOT NULL,
    sender_timestamp bigint,
    device_id integer NOT NULL,
    updated_at bigint DEFAULT (EXTRACT(epoch FROM now()))::bigint NOT NULL
);


--
-- Data for Name: _sqlx_migrations; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public._sqlx_migrations (version, description, installed_on, success, checksum, execution_time) VALUES (1, 'initial', '2026-10-03 11:36:32.583423+00', true, '\xa5e25d232f4c99d1d7931cc06003cd764416ae0af7bfb9c16f3f4fcfa8a79b859048551242164489eaf8dc9e00f5b55d', 77505918);
INSERT INTO public._sqlx_migrations (version, description, installed_on, success, checksum, execution_time) VALUES (2, 'drop connection policy', '2026-10-03 11:36:32.66302+00', true, '\xa21a5c47dbda69ba5aa37eaa8e8c2e859c6f653bcb23e83c766c6da80a1708089c0fcd90ad8da1bffe7f248c86a1b089', 2293071);
INSERT INTO public._sqlx_migrations (version, description, installed_on, success, checksum, execution_time) VALUES (3, 'msg secrets', '2026-10-03 11:36:32.667118+00', true, '\xafbf76e9986c32a2a86d6ee60c1e2372234d7ac6defe6eeb55aaf8a0f762314fe6b1b66e82594f8f4d0f14aec52cf667', 8424336);
INSERT INTO public._sqlx_migrations (version, description, installed_on, success, checksum, execution_time) VALUES (4, 'blob format', '2026-10-03 11:36:32.677354+00', true, '\x5f0001cd7c0bc1b3163417ee139c1f9d3007819fb15e8c39e62c08c486bb55d91afbc663398a982bb670cb8f52de7b8d', 6300822);


--
-- Data for Name: accounts; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.accounts (uuid, external_ref, device_id, push_name, created_at) OVERRIDING SYSTEM VALUE VALUES ('f7d0ffd6-1b78-4963-9a49-814141641187', 'fixture-0f40e34', 1, NULL, 1791027393);


--
-- Data for Name: app_state_keys; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.app_state_keys (key_id, key_data, device_id) VALUES ('\x00000009', '\x0a206666666666666666666666666666666666666666666666666666666666666666120301020318c0f396c206', 1);


--
-- Data for Name: app_state_mutation_macs; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.app_state_mutation_macs (name, version, index_mac, value_mac, device_id) VALUES ('regular_low', 42, '\x1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d', '\x7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a7a', 1);


--
-- Data for Name: app_state_versions; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.app_state_versions (name, state_data, device_id) VALUES ('regular_low', '\x082a1280015a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a1a130a07696e6465782d30120800000000000000001a130a07696e6465782d31120801010101010101011a130a07696e6465782d3212080202020202020202', 1);


--
-- Data for Name: base_keys; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.base_keys (address, message_id, base_key, device_id, created_at) VALUES ('5511900000001.0', 'M1', '\xba005e', 1, 1791027392);


--
-- Data for Name: blob_format; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.blob_format (id, format) VALUES (1, 'protobuf');


--
-- Data for Name: device; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.device (device_id, data, created_at) VALUES (1, '\x0a1f0a0d35353131393030303030303939120e732e77686174736170702e6e657412180a0f31303030303030303030303030393912036c6964200418eeaa85ff0122405855a551d30b263a2c41f7bf679b10c70e5c89ff37a5232a59fb4953ef404a591b5d3fe389da248d4d1911a1c17928a389a3c7cb6284fdb1b492198739b6e3022a4080eece49ff356cb7de2849a6bf0d4e816a28bd341c26f0201d4c05b34234716b68e09450267f0ebec421c90f23788fa30271d395e3a858cfebb88f528b31eb4a3240c0443344993957b269923c0c3f225394f64ab88c43bc41555d79372115c1036f6fc84ae921558da0986b0c999abb9abb305e781a9530b9636cfbb3e28cfd040138034240104fdf3d8d271162eab3e975895587aaf6fdaac285cf571c3947dbf7f643ce5027b6938f44f662ca87df697214cf5bf2b32184c21379d91cece4902019db2c8a4a208716aa6ac048a1aac869af3e3a447f53a765a6a54bdbed5e01311742ff5e11a85a1577616d757820666978747572652030663430653334600268b8177082a0bcf203900109980108a00101b80102c801b8deb48c9034', 1791027393);


--
-- Data for Name: device_registry; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.device_registry (user_id, devices_json, "timestamp", phash, device_id, updated_at, raw_id) VALUES ('5511900000001', '[{"device_id":0,"key_index":null,"is_hosted":false},{"device_id":4,"key_index":0,"is_hosted":true}]', 1749400000, '2:abc', 1, 1791027392, 5);


--
-- Data for Name: identities; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.identities (address, key, device_id) VALUES ('5511900000001.0', '\x1111111111111111111111111111111111111111111111111111111111111111', 1);


--
-- Data for Name: lid_pn_mapping; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.lid_pn_mapping (lid, phone_number, created_at, learning_source, updated_at, device_id) VALUES ('100000000000001', '5511900000001', 1749400000, 'usync', 1749400100, 1);


--
-- Data for Name: msg_secrets; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.msg_secrets (chat, sender, msg_id, secret, expires_at, message_ts, device_id) VALUES ('5511900000001@s.whatsapp.net', '5511900000000@s.whatsapp.net', 'M1', '\x5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c5c', 0, 1749400000, 1);


--
-- Data for Name: prekeys; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.prekeys (id, key, uploaded, device_id) VALUES (7, '\x333333333333333333333333333333333333333333333333', true, 1);
INSERT INTO public.prekeys (id, key, uploaded, device_id) VALUES (8, '\x343434343434343434343434343434343434343434343434', false, 1);


--
-- Data for Name: sender_key_devices; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.sender_key_devices (group_jid, device_jid, has_key, device_id, updated_at) VALUES ('120363000000000001@g.us', '5511900000001:1@s.whatsapp.net', 1, 1, 1791027392);
INSERT INTO public.sender_key_devices (group_jid, device_jid, has_key, device_id, updated_at) VALUES ('120363000000000001@g.us', '5511900000002:3@s.whatsapp.net', 0, 1, 1791027392);


--
-- Data for Name: sender_keys; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.sender_keys (address, record, device_id) VALUES ('120363000000000001@g.us::5511900000001::0', '\x555555555555555555555555555555555555555555555555555555555555', 1);


--
-- Data for Name: sent_messages; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.sent_messages (chat_jid, message_id, payload, device_id, created_at) VALUES ('5511900000001@s.whatsapp.net', 'M1', '\x0a0001', 1, 1791027392);


--
-- Data for Name: sessions; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.sessions (address, record, device_id) VALUES ('5511900000001.0', '\x220022ff22', 1);


--
-- Data for Name: signed_prekeys; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.signed_prekeys (id, record, device_id) VALUES (3, '\x4444444444444444444444444444444444444444444444444444444444444444444444444444444444444444444444444444', 1);


--
-- Data for Name: tc_tokens; Type: TABLE DATA; Schema: public; Owner: -
--

INSERT INTO public.tc_tokens (jid, token, token_timestamp, sender_timestamp, device_id, updated_at) VALUES ('100000000000001@lid', '\x7c00ff', 1749400000, 1749400100, 1, 1791027392);


--
-- Name: accounts_device_id_seq; Type: SEQUENCE SET; Schema: public; Owner: -
--

SELECT pg_catalog.setval('public.accounts_device_id_seq', 1, true);


--
-- Name: _sqlx_migrations _sqlx_migrations_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public._sqlx_migrations
    ADD CONSTRAINT _sqlx_migrations_pkey PRIMARY KEY (version);


--
-- Name: accounts accounts_device_id_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.accounts
    ADD CONSTRAINT accounts_device_id_key UNIQUE (device_id);


--
-- Name: accounts accounts_external_ref_key; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.accounts
    ADD CONSTRAINT accounts_external_ref_key UNIQUE (external_ref);


--
-- Name: accounts accounts_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.accounts
    ADD CONSTRAINT accounts_pkey PRIMARY KEY (uuid);


--
-- Name: app_state_keys app_state_keys_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.app_state_keys
    ADD CONSTRAINT app_state_keys_pkey PRIMARY KEY (key_id, device_id);


--
-- Name: app_state_mutation_macs app_state_mutation_macs_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.app_state_mutation_macs
    ADD CONSTRAINT app_state_mutation_macs_pkey PRIMARY KEY (name, index_mac, device_id);


--
-- Name: app_state_versions app_state_versions_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.app_state_versions
    ADD CONSTRAINT app_state_versions_pkey PRIMARY KEY (name, device_id);


--
-- Name: base_keys base_keys_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.base_keys
    ADD CONSTRAINT base_keys_pkey PRIMARY KEY (address, message_id, device_id);


--
-- Name: blob_format blob_format_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.blob_format
    ADD CONSTRAINT blob_format_pkey PRIMARY KEY (id);


--
-- Name: device device_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.device
    ADD CONSTRAINT device_pkey PRIMARY KEY (device_id);


--
-- Name: device_registry device_registry_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.device_registry
    ADD CONSTRAINT device_registry_pkey PRIMARY KEY (user_id, device_id);


--
-- Name: identities identities_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.identities
    ADD CONSTRAINT identities_pkey PRIMARY KEY (address, device_id);


--
-- Name: lid_pn_mapping lid_pn_mapping_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.lid_pn_mapping
    ADD CONSTRAINT lid_pn_mapping_pkey PRIMARY KEY (lid, device_id);


--
-- Name: msg_secrets msg_secrets_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.msg_secrets
    ADD CONSTRAINT msg_secrets_pkey PRIMARY KEY (chat, sender, msg_id, device_id);


--
-- Name: prekeys prekeys_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.prekeys
    ADD CONSTRAINT prekeys_pkey PRIMARY KEY (id, device_id);


--
-- Name: sender_key_devices sender_key_devices_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sender_key_devices
    ADD CONSTRAINT sender_key_devices_pkey PRIMARY KEY (group_jid, device_jid, device_id);


--
-- Name: sender_keys sender_keys_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sender_keys
    ADD CONSTRAINT sender_keys_pkey PRIMARY KEY (address, device_id);


--
-- Name: sent_messages sent_messages_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sent_messages
    ADD CONSTRAINT sent_messages_pkey PRIMARY KEY (chat_jid, message_id, device_id);


--
-- Name: sessions sessions_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_pkey PRIMARY KEY (address, device_id);


--
-- Name: signed_prekeys signed_prekeys_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.signed_prekeys
    ADD CONSTRAINT signed_prekeys_pkey PRIMARY KEY (id, device_id);


--
-- Name: tc_tokens tc_tokens_pkey; Type: CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tc_tokens
    ADD CONSTRAINT tc_tokens_pkey PRIMARY KEY (jid, device_id);


--
-- Name: idx_device_registry_updated_at; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX idx_device_registry_updated_at ON public.device_registry USING btree (updated_at, device_id);


--
-- Name: idx_lid_pn_mapping_phone; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX idx_lid_pn_mapping_phone ON public.lid_pn_mapping USING btree (phone_number, device_id);


--
-- Name: idx_msg_secrets_expires; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX idx_msg_secrets_expires ON public.msg_secrets USING btree (device_id, expires_at);


--
-- Name: idx_sender_key_devices_group; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX idx_sender_key_devices_group ON public.sender_key_devices USING btree (group_jid, device_id);


--
-- Name: idx_sent_messages_created; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX idx_sent_messages_created ON public.sent_messages USING btree (created_at, device_id);


--
-- Name: idx_tc_tokens_timestamp; Type: INDEX; Schema: public; Owner: -
--

CREATE INDEX idx_tc_tokens_timestamp ON public.tc_tokens USING btree (token_timestamp, device_id);


--
-- Name: app_state_keys app_state_keys_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.app_state_keys
    ADD CONSTRAINT app_state_keys_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- Name: app_state_mutation_macs app_state_mutation_macs_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.app_state_mutation_macs
    ADD CONSTRAINT app_state_mutation_macs_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- Name: app_state_versions app_state_versions_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.app_state_versions
    ADD CONSTRAINT app_state_versions_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- Name: base_keys base_keys_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.base_keys
    ADD CONSTRAINT base_keys_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- Name: device device_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.device
    ADD CONSTRAINT device_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- Name: device_registry device_registry_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.device_registry
    ADD CONSTRAINT device_registry_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- Name: identities identities_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.identities
    ADD CONSTRAINT identities_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- Name: lid_pn_mapping lid_pn_mapping_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.lid_pn_mapping
    ADD CONSTRAINT lid_pn_mapping_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- Name: msg_secrets msg_secrets_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.msg_secrets
    ADD CONSTRAINT msg_secrets_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- Name: prekeys prekeys_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.prekeys
    ADD CONSTRAINT prekeys_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- Name: sender_key_devices sender_key_devices_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sender_key_devices
    ADD CONSTRAINT sender_key_devices_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- Name: sender_keys sender_keys_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sender_keys
    ADD CONSTRAINT sender_keys_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- Name: sent_messages sent_messages_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sent_messages
    ADD CONSTRAINT sent_messages_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- Name: sessions sessions_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.sessions
    ADD CONSTRAINT sessions_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- Name: signed_prekeys signed_prekeys_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.signed_prekeys
    ADD CONSTRAINT signed_prekeys_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- Name: tc_tokens tc_tokens_device_id_fkey; Type: FK CONSTRAINT; Schema: public; Owner: -
--

ALTER TABLE ONLY public.tc_tokens
    ADD CONSTRAINT tc_tokens_device_id_fkey FOREIGN KEY (device_id) REFERENCES public.accounts(device_id) ON DELETE CASCADE;


--
-- PostgreSQL database dump complete
--


