import importlib  # noqa: I001
import re
import io
import os
import socket
import subprocess
import tempfile
import time
import uuid
from collections.abc import Generator
from contextlib import contextmanager
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone
from pathlib import Path

import httpx
import pytest
from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import rsa
from cryptography.x509.oid import NameOID

ROOT = Path(__file__).resolve().parents[2]
OPENAPI_PATH = ROOT / "openapi.json"
WEBSITES_BIN = ROOT / "target" / "debug" / "websites"
SESSION_SEED_BIN = ROOT / "target" / "debug" / "session_seed"

TINY_PNG_BYTES = bytes(
    [
        0x89,
        0x50,
        0x4E,
        0x47,
        0x0D,
        0x0A,
        0x1A,
        0x0A,
        0x00,
        0x00,
        0x00,
        0x0D,
        0x49,
        0x48,
        0x44,
        0x52,
        0x00,
        0x00,
        0x00,
        0x01,
        0x00,
        0x00,
        0x00,
        0x01,
        0x08,
        0x06,
        0x00,
        0x00,
        0x00,
        0x1F,
        0x15,
        0xC4,
        0x89,
        0x00,
        0x00,
        0x00,
        0x0D,
        0x49,
        0x44,
        0x41,
        0x54,
        0x78,
        0x9C,
        0x63,
        0xF8,
        0xFF,
        0xFF,
        0xFF,
        0x7F,
        0x00,
        0x09,
        0xFB,
        0x03,
        0xFD,
        0x28,
        0xA6,
        0xE3,
        0x8A,
        0x00,
        0x00,
        0x00,
        0x00,
        0x49,
        0x45,
        0x4E,
        0x44,
        0xAE,
        0x42,
        0x60,
        0x82,
    ]
)


def run_command(args: list[str], *, env: dict[str, str] | None = None) -> str:
    result = subprocess.run(
        args,
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        check=True,
    )
    return result.stdout.strip()


def parse_created_id(output: str) -> uuid.UUID:
    try:
        return uuid.UUID(output)  # validate it's a uuid
    except ValueError:
        raise AssertionError(f"output is not a valid UUID: {output}")


def find_created_id(output: str) -> uuid.UUID:
    matcher = re.compile("created site: (?P<site_id>[0-9a-f\-]+)", re.IGNORECASE | re.MULTILINE)
    match = matcher.search(output)
    if not match:
        raise AssertionError(f"failed to parse created id from output: {output}")
    return parse_created_id(match.group("site_id"))


def find_uuid(output: str) -> uuid.UUID:
    matcher = re.compile("([0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12})", re.IGNORECASE)
    match = matcher.search(output)
    if not match:
        raise AssertionError(f"failed to find UUID in output: {output}")
    return parse_created_id(match.group(1))


def test_parse_created_id() -> None:
    """this is a bit recursive"""
    with pytest.raises(AssertionError, match="^output is not a valid UUID"):
        parse_created_id("not-a-uuid")
    with pytest.raises(AssertionError, match="^output is not a valid UUID"):
        parse_created_id("Created new thing with id: 12345")
    valid_uuid = str(uuid.uuid4())
    assert parse_created_id(f"Created new thing with id: {valid_uuid}".split()[-1]) == uuid.UUID(valid_uuid)


def reserve_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        sock.listen(1)
        return int(sock.getsockname()[1])


def wait_for_port(port: int, process: subprocess.Popen[str], timeout: float = 20.0) -> None:
    started = time.time()
    while time.time() - started < timeout:
        if process.poll() is not None:
            stdout, stderr = process.communicate(timeout=1)
            raise AssertionError(f"server exited early\nstdout:\n{stdout}\nstderr:\n{stderr}")
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.5):
                return
        except OSError:
            time.sleep(0.2)
    stdout, stderr = process.communicate(timeout=1)
    raise AssertionError(f"server did not become ready\nstdout:\n{stdout}\nstderr:\n{stderr}")


def generate_tls_material(temp_root: Path) -> tuple[Path, Path]:
    key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
    subject = issuer = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, "127.0.0.1")])
    san = x509.SubjectAlternativeName(
        [
            x509.IPAddress(__import__("ipaddress").ip_address("127.0.0.1")),
            x509.DNSName("localhost"),
        ]
    )
    now = datetime.now(timezone.utc)
    cert = (
        x509.CertificateBuilder()
        .subject_name(subject)
        .issuer_name(issuer)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now - timedelta(minutes=5))
        .not_valid_after(now + timedelta(days=7))
        .add_extension(san, critical=False)
        .sign(key, hashes.SHA256())
    )
    cert_path = temp_root / "tls.crt"
    key_path = temp_root / "tls.key"
    cert_path.write_bytes(cert.public_bytes(serialization.Encoding.PEM))
    key_path.write_bytes(
        key.private_bytes(
            encoding=serialization.Encoding.PEM,
            format=serialization.PrivateFormat.TraditionalOpenSSL,
            encryption_algorithm=serialization.NoEncryption(),
        )
    )
    return cert_path, key_path


@dataclass
class Harness:
    base_url: str
    site_id: uuid.UUID
    session_id: str
    upload_root: Path
    process: subprocess.Popen[str]


def build_session_client(client_module, harness: Harness):
    return client_module.AuthenticatedClient(
        base_url=harness.base_url,
        token="unused-session-token",
        verify_ssl=False,
        raise_on_unexpected_status=True,
    ).set_httpx_client(
        httpx.Client(
            base_url=harness.base_url,
            cookies={"id": harness.session_id},
            verify=False,
        )
    )


@contextmanager
def running_server() -> Generator[Harness, None, None]:
    with tempfile.TemporaryDirectory(prefix="websites-api-client-") as tmp_dir:
        temp_root = Path(tmp_dir)
        db_path = temp_root / "database.sqlite"
        upload_root = temp_root / "uploads"
        cert_path, key_path = generate_tls_material(temp_root)

        site_id = find_created_id(
            run_command(
                [
                    str(WEBSITES_BIN),
                    "--database-url",
                    str(db_path),
                    "site",
                    "create",
                    "--short-name",
                    "py-client",
                    "--full-title",
                    "Python Client Test",
                    "--template-name",
                    "default",
                ]
            )
        )

        user_id = find_uuid(
            run_command(
                [
                    str(WEBSITES_BIN),
                    "--database-url",
                    str(db_path),
                    "user",
                    "create",
                    "--subject",
                    "py-client-author",
                ]
            )
        )
        run_command(
            [
                str(WEBSITES_BIN),
                "--database-url",
                str(db_path),
                "site",
                "member-add",
                "--site-id",
                str(site_id),
                "--user-id",
                str(user_id),
                "--role",
                "author",
            ]
        )

        port = reserve_port()
        base_url = f"https://127.0.0.1:{port}"
        database_url = f"sqlite://{db_path}?mode=rwc"
        env = os.environ.copy()
        env["WEBSITES_UPLOAD_ROOT"] = str(upload_root)
        process = subprocess.Popen(
            [
                str(WEBSITES_BIN),
                "--database-url",
                str(db_path),
                "--client-id",
                "pytest-client",
                "--discovery-url",
                "https://example.com/.well-known/openid-configuration",
                "--frontend-url",
                base_url,
                "--tls-cert-path",
                str(cert_path),
                "--tls-key-path",
                str(key_path),
                "serve",
                "admin",
                "--listen",
                f"127.0.0.1:{port}",
            ],
            cwd=ROOT,
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        try:
            wait_for_port(port, process)
            session_id = run_command(
                [
                    str(SESSION_SEED_BIN),
                    "--database-url",
                    database_url,
                    "--user-sub",
                    "py-client-author",
                ],
                env=env,
            ).strip()
            yield Harness(
                base_url=base_url,
                site_id=site_id,
                session_id=session_id,
                upload_root=upload_root,
                process=process,
            )
        finally:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=10)


def test_generated_client_crud_and_search():
    with running_server() as harness:
        client_module = importlib.import_module("websites.client")
        content_create_module = importlib.import_module("websites.api.default.api_site_content_create")
        content_get_module = importlib.import_module("websites.api.default.api_site_content_get")
        content_list_module = importlib.import_module("websites.api.default.api_site_content_list")
        content_search_module = importlib.import_module("websites.api.default.api_site_content_search")
        content_update_module = importlib.import_module("websites.api.default.api_site_content_update")
        content_delete_module = importlib.import_module("websites.api.default.api_site_content_delete")
        asset_create_module = importlib.import_module("websites.api.default.api_site_asset_create")
        asset_get_module = importlib.import_module("websites.api.default.api_site_asset_get")
        asset_list_module = importlib.import_module("websites.api.default.api_site_assets_list")
        asset_library_module = importlib.import_module("websites.api.default.api_site_assets_library")
        asset_delete_module = importlib.import_module("websites.api.default.api_site_asset_delete")
        create_content_model = importlib.import_module("websites.models.api_create_content_request")
        update_content_model = importlib.import_module("websites.models.api_update_content_request")
        error_response_model = importlib.import_module("websites.models.api_error_response")
        asset_upload_model = importlib.import_module("websites.models.asset_upload_request")
        types_module = importlib.import_module("websites.types")

        with build_session_client(client_module, harness) as client:
            created_content = content_create_module.sync(
                site_id=harness.site_id,
                client=client,
                body=create_content_model.ApiCreateContentRequest(
                    draft=True,
                    page_content="Python client body",
                    page_type="page",
                    slug="python-client-body",
                    title="Python Client Body",
                    tags=["alpha", "beta"],
                ),
            )
            assert created_content is not None
            assert not isinstance(created_content, error_response_model.ApiErrorResponse)
            content_id = created_content.id
            assert created_content.title == "Python Client Body"
            assert created_content.tags == ["alpha", "beta"]

            listed = content_list_module.sync(site_id=harness.site_id, client=client)
            assert listed is not None
            assert not isinstance(listed, error_response_model.ApiErrorResponse)
            assert any(item.id == content_id for item in listed.items)

            fetched = content_get_module.sync(
                site_id=harness.site_id,
                content_id=content_id,
                client=client,
            )
            assert fetched is not None
            assert not isinstance(fetched, error_response_model.ApiErrorResponse)
            assert fetched.page_content == "Python client body"

            updated = content_update_module.sync(
                site_id=harness.site_id,
                content_id=content_id,
                client=client,
                body=update_content_model.ApiUpdateContentRequest(
                    title="Python Client Published",
                    draft=False,
                    tags=["gamma"],
                ),
            )
            assert updated is not None
            assert not isinstance(updated, error_response_model.ApiErrorResponse)
            assert updated.title == "Python Client Published"
            assert updated.tags == ["gamma"]

            searched = content_search_module.sync(
                site_id=harness.site_id,
                client=client,
                q="Published",
            )
            assert searched is not None
            assert not isinstance(searched, error_response_model.ApiErrorResponse)
            assert len(searched.items) == 1
            assert searched.items[0].id == content_id

            uploaded_asset = asset_create_module.sync(
                site_id=harness.site_id,
                client=client,
                body=asset_upload_model.AssetUploadRequest(
                    file=[
                        types_module.File(
                            payload=io.BytesIO(TINY_PNG_BYTES),
                            file_name="tiny.png",
                            mime_type="image/png",
                        ),
                        types_module.File(
                            payload=io.BytesIO(TINY_PNG_BYTES),
                            file_name="tiny-2.png",
                            mime_type="image/png",
                        ),
                    ]
                ),
            )
            assert uploaded_asset is not None
            assert not isinstance(uploaded_asset, error_response_model.ApiErrorResponse)
            assert len(uploaded_asset.assets) == 2
            asset_id = uploaded_asset.assets[0].id
            second_asset_id = uploaded_asset.assets[1].id
            assert uploaded_asset.assets[0].original_filename == "tiny.png"
            assert uploaded_asset.assets[1].original_filename == "tiny-2.png"

            stored_files = sorted(path.name for path in harness.upload_root.iterdir())
            assert stored_files

            listed_assets = asset_list_module.sync(site_id=harness.site_id, client=client)
            assert listed_assets is not None
            assert not isinstance(listed_assets, error_response_model.ApiErrorResponse)
            assert any(asset.id == asset_id for asset in listed_assets.assets)
            assert any(asset.original_filename == "tiny-2.png" for asset in listed_assets.assets)

            library_assets = asset_library_module.sync(site_id=harness.site_id, client=client)
            assert library_assets is not None
            assert not isinstance(library_assets, error_response_model.ApiErrorResponse)
            assert any(asset.id == asset_id for asset in library_assets.assets)

            fetched_asset = asset_get_module.sync(
                site_id=harness.site_id,
                asset_id=asset_id,
                client=client,
            )
            assert fetched_asset is not None
            assert not isinstance(fetched_asset, error_response_model.ApiErrorResponse)
            assert fetched_asset.asset.original_filename == "tiny.png"
            assert fetched_asset.asset.variants

            delete_asset_response = asset_delete_module.sync_detailed(
                site_id=harness.site_id,
                asset_id=asset_id,
                client=client,
            )
            assert delete_asset_response.status_code == 204

            delete_second_asset_response = asset_delete_module.sync_detailed(
                site_id=harness.site_id,
                asset_id=second_asset_id,
                client=client,
            )
            assert delete_second_asset_response.status_code == 204
            assert list(harness.upload_root.iterdir()) == []

            delete_content_response = content_delete_module.sync_detailed(
                site_id=harness.site_id,
                content_id=content_id,
                client=client,
            )
            assert delete_content_response.status_code == 204

            missing_content = content_get_module.sync_detailed(
                site_id=harness.site_id,
                content_id=content_id,
                client=client,
            )
            assert missing_content.status_code == 404
