from http import HTTPStatus
from typing import Any
from urllib.parse import quote
from uuid import UUID

import httpx

from ... import errors
from ...client import AuthenticatedClient, Client
from ...models.api_asset_create_response import ApiAssetCreateResponse
from ...models.api_error_response import ApiErrorResponse
from ...models.asset_upload_request import AssetUploadRequest
from ...types import Response


def _get_kwargs(
    site_id: UUID,
    *,
    body: AssetUploadRequest,
) -> dict[str, Any]:
    headers: dict[str, Any] = {}

    _kwargs: dict[str, Any] = {
        "method": "post",
        "url": "/api/site/{site_id}/assets".format(
            site_id=quote(str(site_id), safe=""),
        ),
    }

    _kwargs["files"] = body.to_multipart()

    _kwargs["headers"] = headers
    return _kwargs


def _parse_response(
    *, client: AuthenticatedClient | Client, response: httpx.Response
) -> ApiAssetCreateResponse | ApiErrorResponse | None:
    if response.status_code == 200:
        response_200 = ApiAssetCreateResponse.from_dict(response.json())

        return response_200

    if response.status_code == 400:
        response_400 = ApiErrorResponse.from_dict(response.json())

        return response_400

    if response.status_code == 401:
        response_401 = ApiErrorResponse.from_dict(response.json())

        return response_401

    if response.status_code == 403:
        response_403 = ApiErrorResponse.from_dict(response.json())

        return response_403

    if response.status_code == 500:
        response_500 = ApiErrorResponse.from_dict(response.json())

        return response_500

    if client.raise_on_unexpected_status:
        raise errors.UnexpectedStatus(response.status_code, response.content)
    else:
        return None


def _build_response(
    *, client: AuthenticatedClient | Client, response: httpx.Response
) -> Response[ApiAssetCreateResponse | ApiErrorResponse]:
    return Response(
        status_code=HTTPStatus(response.status_code),
        content=response.content,
        headers=response.headers,
        parsed=_parse_response(client=client, response=response),
    )


def sync_detailed(
    site_id: UUID,
    *,
    client: AuthenticatedClient,
    body: AssetUploadRequest,
) -> Response[ApiAssetCreateResponse | ApiErrorResponse]:
    """
    Args:
        site_id (UUID):
        body (AssetUploadRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ApiAssetCreateResponse | ApiErrorResponse]
    """

    kwargs = _get_kwargs(
        site_id=site_id,
        body=body,
    )

    response = client.get_httpx_client().request(
        **kwargs,
    )

    return _build_response(client=client, response=response)


def sync(
    site_id: UUID,
    *,
    client: AuthenticatedClient,
    body: AssetUploadRequest,
) -> ApiAssetCreateResponse | ApiErrorResponse | None:
    """
    Args:
        site_id (UUID):
        body (AssetUploadRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ApiAssetCreateResponse | ApiErrorResponse
    """

    return sync_detailed(
        site_id=site_id,
        client=client,
        body=body,
    ).parsed


async def asyncio_detailed(
    site_id: UUID,
    *,
    client: AuthenticatedClient,
    body: AssetUploadRequest,
) -> Response[ApiAssetCreateResponse | ApiErrorResponse]:
    """
    Args:
        site_id (UUID):
        body (AssetUploadRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        Response[ApiAssetCreateResponse | ApiErrorResponse]
    """

    kwargs = _get_kwargs(
        site_id=site_id,
        body=body,
    )

    response = await client.get_async_httpx_client().request(**kwargs)

    return _build_response(client=client, response=response)


async def asyncio(
    site_id: UUID,
    *,
    client: AuthenticatedClient,
    body: AssetUploadRequest,
) -> ApiAssetCreateResponse | ApiErrorResponse | None:
    """
    Args:
        site_id (UUID):
        body (AssetUploadRequest):

    Raises:
        errors.UnexpectedStatus: If the server returns an undocumented status code and Client.raise_on_unexpected_status is True.
        httpx.TimeoutException: If the request takes longer than Client.timeout.

    Returns:
        ApiAssetCreateResponse | ApiErrorResponse
    """

    return (
        await asyncio_detailed(
            site_id=site_id,
            client=client,
            body=body,
        )
    ).parsed
