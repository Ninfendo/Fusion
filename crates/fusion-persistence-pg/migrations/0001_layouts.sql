-- Window layouts by name, as the JSON the client sent.
CREATE TABLE layouts (
    name     text        PRIMARY KEY,
    layout   text        NOT NULL CHECK (layout IS JSON OBJECT),
    saved_at timestamptz NOT NULL DEFAULT now()
);
